// 8-channel SUMP-compatible logic analyzer for the Mojo V3 (Spartan-6 XC6SLX9).
// Host side: PulseView / sigrok-cli, driver "ols" (Openbench Logic Sniffer),
// over the Mojo's USB serial port (the AVR bridges it to us at 500 kbaud).
module mojo_top (
    input        clk,          // 50 MHz
    input        rst_n,
    input        cclk,
    output [7:0] led,
    output       spi_miso,
    output [3:0] spi_channel,
    input        avr_tx,       // AVR -> FPGA serial
    output       avr_rx,       // FPGA -> AVR serial
    input        avr_rx_busy,
    input  [7:0] probe
);
    localparam AW = 14;                         // 16K samples (8 of 32 BRAMs)
    localparam [31:0] DEPTH = 1 << AW;
    localparam [31:0] MAX_RATE = 50_000_000;
    localparam CLK_PER_BIT = 100;               // 500 kbaud, fixed by the Mojo AVR firmware

    wire rst = ~rst_n;

    // The AVR shares these pins while configuring us; wait for cclk to stay high.
    reg [12:0] cclk_cnt = 0;
    wire ready = &cclk_cnt;
    always @(posedge clk)
        if (rst || !cclk) cclk_cnt <= 0;
        else if (!ready) cclk_cnt <= cclk_cnt + 1;

    wire tx;
    assign avr_rx      = ready ? tx : 1'bz;
    assign spi_miso    = 1'bz;
    assign spi_channel = 4'bzzzz;

    reg [7:0] p1, in;                           // 2-FF synchronizer
    always @(posedge clk) begin p1 <= probe; in <= p1; end
    assign led = in;                            // live view of the probes

    wire [7:0] rx_data;
    wire       rx_new;
    uart_rx #(CLK_PER_BIT) urx (clk, rst, avr_tx, rx_data, rx_new);

    reg        tx_start = 0;
    reg  [7:0] tx_data = 0;
    wire       tx_busy;
    uart_tx #(CLK_PER_BIT) utx (clk, rst, tx_start, tx_data, tx, tx_busy);
    // Flow control: the Mojo AVR firmware toggles avr_rx_busy once per byte it has
    // room for, and stops toggling when its buffer is nearly full. So it's a
    // credit line, not a level: send one byte per edge.
    reg [2:0] busy_s = 0;
    reg can_send = 1;
    always @(posedge clk) begin
        busy_s <= {busy_s[1:0], avr_rx_busy};
        if (tx_start) can_send <= 0;
        else if (busy_s[2] ^ busy_s[1]) can_send <= 1;
    end
    wire tx_ok = ready && !tx_busy && !tx_start && can_send;

    // 0..3: ID reply, 4..27: metadata (strings null-terminated, 32-bit values big-endian)
    function [7:0] rom(input [4:0] a);
        case (a)
            0: rom = "1";  1: rom = "A";  2: rom = "L";  3: rom = "S";
            4: rom = 8'h01;                             // device name
            5: rom = "M";  6: rom = "o";  7: rom = "j";  8: rom = "o";
            9: rom = " "; 10: rom = "L"; 11: rom = "A"; 12: rom = 0;
            13: rom = 8'h21;                            // sample memory
            14: rom = DEPTH[31:24];    15: rom = DEPTH[23:16];
            16: rom = DEPTH[15:8];     17: rom = DEPTH[7:0];
            18: rom = 8'h23;                            // max sample rate
            19: rom = MAX_RATE[31:24]; 20: rom = MAX_RATE[23:16];
            21: rom = MAX_RATE[15:8];  22: rom = MAX_RATE[7:0];
            23: rom = 8'h40; 24: rom = 8;               // probes
            25: rom = 8'h41; 26: rom = 2;               // protocol version
            default: rom = 0;                           // 27: end
        endcase
    endfunction

    localparam IDLE = 0, ROM = 1, ARMED = 2, POST = 3, LOAD = 4, SEND = 5;
    reg [2:0]  state = IDLE;
    reg [4:0]  rp = 0, rend = 0;
    reg [2:0]  nargs = 0;
    reg        do_long = 0;
    reg [7:0]  cmd = 0;
    reg [31:0] arg = 0;
    reg [23:0] div = 0, tick_cnt = 0;
    reg [15:0] rcount = 0, dcount = 0;          // in units of 4 samples, minus 1
    reg [7:0]  tmask = 0, tval = 0;
    reg [AW-1:0] wa = 0, ra = 0;
    reg [18:0] n = 0;

    // SUMP divider is relative to 100 MHz; we run at 50, so reload with div/2.
    wire tick = tick_cnt == 0;
    wire trig = ((in ^ tval) & tmask) == 0;
    wire we   = tick && (state == ARMED || state == POST);

    // Hold off the trigger until the buffer has the requested pre-trigger samples,
    // or the start of the capture would be stale data from the previous run.
    reg  [18:0] filled = 0;
    wire [18:0] pre_need = (rcount > dcount) ? (rcount - dcount) * 4 : 0;

    reg [7:0] mem [0:DEPTH-1];
    reg [7:0] q;
    always @(posedge clk) begin
        if (we) mem[wa] <= in;
        q <= mem[ra];
    end

    always @(posedge clk) begin
        tx_start <= 0;
        do_long  <= 0;
        tick_cnt <= tick ? div >> 1 : tick_cnt - 1;
        if (we) wa <= wa + 1;
        if (we && state == ARMED && filled != DEPTH) filled <= filled + 1;

        if (rst) begin
            state <= IDLE;
            nargs <= 0;
        end else begin
            case (state)
                ROM: if (tx_ok) begin
                    tx_data  <= rom(rp);
                    tx_start <= 1;
                    rp       <= rp + 1;
                    if (rp == rend) state <= IDLE;
                end
                ARMED: if (tick && trig && filled >= pre_need) begin
                    n     <= (dcount + 1) * 4;
                    state <= POST;
                end
                POST: if (tick) begin
                    n <= n - 1;
                    if (n == 1) begin
                        ra    <= wa;            // last sample written this cycle
                        n     <= (rcount + 1) * 4;
                        state <= LOAD;
                    end
                end
                LOAD: state <= SEND;            // wait out BRAM read latency
                SEND: if (tx_ok) begin          // SUMP sends newest sample first
                    tx_data  <= q;
                    tx_start <= 1;
                    ra       <= ra - 1;
                    n        <= n - 1;
                    state    <= (n == 1) ? IDLE : LOAD;
                end
            endcase

            // Commands: bit 7 clear = 1 byte; set = 1 byte + 4 little-endian arg bytes.
            if (do_long) case (cmd)
                8'h80: div <= arg[23:0];
                8'h81: {dcount, rcount} <= arg;
                8'hC0: tmask <= arg[7:0];
                8'hC1: tval  <= arg[7:0];
                default: ;                      // flags, other trigger stages: ignored
            endcase

            if (rx_new) begin
                if (nargs != 0) begin
                    arg     <= {rx_data, arg[31:8]};
                    nargs   <= nargs - 1;
                    do_long <= nargs == 1;
                end else if (rx_data[7]) begin
                    cmd   <= rx_data;
                    nargs <= 4;
                end else case (rx_data)
                    8'h00: state <= IDLE;                                   // reset
                    8'h01: begin state <= ARMED; tick_cnt <= 0; filled <= 0; end  // run
                    8'h02: begin state <= ROM; rp <= 0; rend <= 3;  end     // ID
                    8'h04: begin state <= ROM; rp <= 4; rend <= 27; end     // metadata
                    default: ;
                endcase
            end
        end
    end
endmodule

module uart_rx #(parameter CPB = 100) (
    input clk, rst, rx_in,
    output reg [7:0] data = 0,
    output reg new_data = 0
);
    reg r1 = 1, rx = 1, busy = 0;
    reg [15:0] cnt = 0;
    reg [3:0] bitn = 0;
    always @(posedge clk) begin
        r1 <= rx_in; rx <= r1;
        new_data <= 0;
        if (rst) busy <= 0;
        else if (!busy) begin
            if (!rx) begin busy <= 1; cnt <= CPB / 2; bitn <= 0; end
        end else if (cnt != 0) cnt <= cnt - 1;
        else begin
            cnt  <= CPB - 1;
            bitn <= bitn + 1;
            if (bitn == 0) begin if (rx) busy <= 0; end    // glitch, not a start bit
            else if (bitn <= 8) data <= {rx, data[7:1]};
            else begin busy <= 0; new_data <= rx; end       // drop on framing error
        end
    end
endmodule

module uart_tx #(parameter CPB = 100) (
    input clk, rst, start,
    input [7:0] data,
    output tx, busy
);
    reg [9:0] sh = 10'h3FF;
    reg [15:0] cnt = 0;
    reg [3:0] left = 0;
    assign tx   = sh[0];
    assign busy = left != 0;
    always @(posedge clk)
        if (rst) begin left <= 0; sh <= 10'h3FF; end
        else if (start && !busy) begin sh <= {1'b1, data, 1'b0}; left <= 10; cnt <= CPB - 1; end
        else if (busy) begin
            if (cnt != 0) cnt <= cnt - 1;
            else begin cnt <= CPB - 1; sh <= {1'b1, sh[9:1]}; left <= left - 1; end
        end
endmodule
