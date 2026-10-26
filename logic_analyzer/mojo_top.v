// 16-channel SUMP-compatible logic analyzer for the Mojo V3 (Spartan-6 XC6SLX9).
// Host side: PulseView / sigrok-cli, driver "ols" (Openbench Logic Sniffer),
// over the Mojo's USB serial port (the AVR bridges it to us at 500 kbaud).
module mojo_top #(
    parameter AW = 14,                          // 16K samples x 16 channels (16 of 32 BRAMs); the testbench shrinks it
    parameter LW = 11                           // live-log FIFO: 2K records (2 more BRAMs; 8K spread the 200 MHz placement too thin)
) (
    input        clk,          // 50 MHz crystal; everything runs on the PLL's 200 MHz (sclk)
    input        rst_n,
    input        cclk,
    output [7:0] led,
    output       spi_miso,
    output [3:0] spi_channel,
    input        avr_tx,       // AVR -> FPGA serial
    output       avr_rx,       // FPGA -> AVR serial
    input        avr_rx_busy,
    input [15:0] probe
);
    localparam [31:0] DEPTH = 1 << AW;
    // Reported to SUMP hosts: rates up to 100 MHz keep their standard meaning. 200 MHz
    // needs the demux flag, which only Mojo LA sends.
    localparam [31:0] MAX_RATE = 100_000_000;
    localparam CLK_PER_BIT = 400;               // 500 kbaud at 200 MHz, fixed by the Mojo AVR firmware

    // PLL: 50 MHz crystal x16 = 800 MHz VCO, /4 = 200 MHz for the whole design, sampling included.
    wire clk_fb, clk200, locked, sclk;
    PLL_BASE #(
        .CLKIN_PERIOD(20.0), .CLKFBOUT_MULT(16), .CLKOUT0_DIVIDE(4), .COMPENSATION("INTERNAL")
    ) pll (
        .CLKIN(clk), .CLKFBIN(clk_fb), .CLKFBOUT(clk_fb), .RST(1'b0), .LOCKED(locked),
        .CLKOUT0(clk200), .CLKOUT1(), .CLKOUT2(), .CLKOUT3(), .CLKOUT4(), .CLKOUT5()
    );
    BUFG sclk_buf (.I(clk200), .O(sclk));

    wire rst = ~rst_n | ~locked;

    // The AVR shares these pins while configuring us; wait for cclk to stay high.
    // The top bit is `ready`: one flip-flop instead of a 13-input AND in the 200 MHz paths.
    reg [13:0] cclk_cnt = 0;
    wire ready = cclk_cnt[13];
    always @(posedge sclk)
        if (rst || !cclk) cclk_cnt <= 0;
        else if (!ready) cclk_cnt <= cclk_cnt + 1;

    wire tx;
    assign avr_rx      = ready ? tx : 1'bz;
    assign spi_miso    = 1'bz;
    assign spi_channel = 4'bzzzz;

    reg [15:0] p1, in;                          // 2-FF synchronizer
    always @(posedge sclk) begin p1 <= probe; in <= p1; end
    // LEDs show activity: every edge on probe N or N+8 lights LED N for BLINK cycles,
    // so traffic too fast to see (a UART, SPI) shows up as a visible blink.
    localparam BLINK = 10_000_000;              // 50 ms at 200 MHz; tune to taste
    reg [15:0] in_d;
    always @(posedge sclk) in_d <= in;
    genvar g;
    generate for (g = 0; g < 8; g = g + 1) begin : activity
        reg [23:0] t = 0;
        always @(posedge sclk)
            if ((in[g] ^ in_d[g]) | (in[g + 8] ^ in_d[g + 8])) t <= BLINK;
            else if (t != 0)     t <= t - 1;
        assign led[g] = t != 0;
    end endgenerate

    wire [7:0] rx_data;
    wire       rx_new;
    uart_rx #(CLK_PER_BIT) urx (sclk, rst, avr_tx, rx_data, rx_new);

    reg        tx_start = 0;
    reg  [7:0] tx_data = 0;
    wire       tx_busy;
    uart_tx #(CLK_PER_BIT) utx (sclk, rst, tx_start, tx_data, tx, tx_busy);
    // Flow control: the Mojo AVR firmware toggles avr_rx_busy once per byte it has
    // room for, and stops toggling when its buffer is nearly full. So it's a
    // credit line, not a level: send one byte per edge.
    reg [2:0] busy_s = 0;
    reg can_send = 1;
    always @(posedge sclk) begin
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
            23: rom = 8'h40; 24: rom = 16;              // probes
            25: rom = 8'h41; 26: rom = 2;               // protocol version
            default: rom = 0;                           // 27: end
        endcase
    endfunction

    localparam IDLE = 0, ROM = 1, ARMED = 2, POST = 3, LOAD = 4, SEND = 5, LOAD0 = 6, LIVE = 7;
    reg [2:0]  state = IDLE;
    reg [4:0]  rp = 0, rend = 0;
    reg [2:0]  nargs = 0;
    reg        do_long = 0;
    reg [7:0]  cmd = 0;
    reg [31:0] arg = 0;
    reg [23:0] div = 0;
    reg [24:0] tick_cnt = 0;
    reg        demux = 0;                       // SUMP flag bit 0: divider counts 200 MHz, not 100 MHz
    reg [15:0] rcount = 0, dcount = 0;          // in units of 4 samples, minus 1
    reg [15:0] tmask = 0, tval = 0;
    reg [1:0]  groups = 2'b11;                  // SUMP channel groups to send (flags bits 2/3 disable them)
    reg [AW-1:0] wa = 0, ra = 0;
    reg [18:0] n = 0;
    reg        n_one = 0;                       // n == 1, kept registered for 200 MHz timing

    // A sample every (div+1) periods of SUMP's 100 MHz, so 2*(div+1) of our 200 MHz cycles;
    // in demux mode the divider counts our 200 MHz directly (div 0 = every cycle).
    // tick is registered (predicted a cycle ahead) so no 25-bit compare sits in the 200 MHz paths.
    reg  tick = 1;
    reg  every = 0;                             // a sample every cycle: 200 MHz
    wire [24:0] reload = demux ? {1'b0, div} : {div, 1'b1};
    always @(posedge sclk) every <= demux && div == 0;
    // One pipeline stage for 200 MHz: the sample and whether it matches the trigger,
    // registered together so they stay aligned.
    reg [15:0] samp = 0;
    reg       trig = 0;
    always @(posedge sclk) begin
        samp <= in;
        trig <= ((in ^ tval) & tmask) == 0;
    end
    // Long captures (flags bit 14, 8 channels): store an entry only when a run of equal samples ends,
    // as {samples it lasted - 1, value}; a run longer than 256 samples becomes several entries.
    // ponytail: the run in progress when the capture ends isn't stored, so a capture ends at its last change
    reg        rle = 0;
    reg  [7:0] rl_val = 0, rl_cnt = 0;
    reg        rl_first = 1;
    reg        rl_full = 0;                     // rl_cnt == 255, registered a count early for 200 MHz timing
    wire       capturing = state == ARMED || state == POST;
    wire       rl_end = !rl_first && (samp[7:0] != rl_val || rl_full);
    // wstop ends writing exactly at the last post-trigger entry. POST notices a cycle later (it counts
    // registered writes), and at 200 MHz another write in that cycle would overwrite the oldest entry.
    reg        wstop = 0, wleft1 = 0;
    reg [18:0] wleft = 0;                       // post-trigger writes still allowed
    wire       we   = capturing && tick && (!rle || rl_end) && !wstop;
    wire [15:0] wdata = rle ? {rl_cnt, rl_val} : samp;

    // Hold off the trigger until the buffer has the requested pre-trigger samples,
    // or the start of the capture would be stale data from the previous run.
    // Both steps are registered so neither sits in one 5 ns path; they settle long before a capture.
    reg  [18:0] filled = 0;
    reg  [18:0] pre_need = 0;
    reg         pre_ok = 0;
    always @(posedge sclk) pre_need <= (rcount > dcount) ? (rcount - dcount) * 4 : 0;

    // The write lands one cycle late, from registers, so the BRAM write enable comes straight off a flip-flop.
    // POST counts these registered writes too (wp_r: written while in POST), keeping the run compare
    // out of the state machine's paths.
    reg        we_r = 0, wp_r = 0;
    reg [15:0] wdata_r = 0;
    reg [AW-1:0] wa_r = 0;
    always @(posedge sclk) begin we_r <= we; wp_r <= we && state == POST; wdata_r <= wdata; wa_r <= wa; end
    reg [15:0] mem [0:DEPTH-1];
    reg [15:0] q;
    reg        hi_byte = 0;                     // sending a sample's second group
    always @(posedge sclk) begin
        if (we_r) mem[wa_r] <= wdata_r;
        q <= mem[ra];
    end

    // Live UART logging (commands 0x90/0x91 set up the receivers, 0x03 starts): bytes stream as
    // 2-byte records from a FIFO instead of being sampled. See live_log below.
    reg  [3:0]  lch0 = 0, lch1 = 1;
    reg         len0 = 0, len1 = 0;
    reg  [19:0] lcpb0 = 1736, lcpb1 = 1736;     // clocks per bit at 200 MHz: 115200 baud; 20 bits reach 191 baud
    reg         lv_pop = 0, lv_second = 0;
    wire [15:0] lv_front;
    wire        lv_avail;
    live_log #(.LW(LW)) live (.clk(sclk), .run(state == LIVE), .in(in),
        .ch0(lch0), .ch1(lch1), .en0(len0), .en1(len1), .cpb0(lcpb0), .cpb1(lcpb1),
        .pop(lv_pop), .front(lv_front), .avail(lv_avail));

    // The next byte to send, picked a cycle ahead: block RAM to the UART in one 5 ns cycle doesn't
    // route. steady says out_b's inputs held still last cycle (each send is followed by a whole byte
    // time of tx_busy, so waiting one more cycle first costs nothing).
    reg  [7:0]  out_b = 0;
    reg         steady = 0;
    always @(posedge sclk) begin
        out_b  <= state == LIVE ? (lv_second ? lv_front[7:0] : lv_front[15:8]) : hi_byte ? q[15:8] : q[7:0];
        steady <= state == SEND || state == LIVE && lv_avail && !lv_pop;
    end

    always @(posedge sclk) begin
        tx_start <= 0;
        do_long  <= 0;
        lv_pop   <= 0;
        if (tick) begin tick_cnt <= reload;       tick <= every;         end
        else      begin tick_cnt <= tick_cnt - 1; tick <= tick_cnt == 1; end
        if (we) wa <= wa + 1;
        if (we && state == POST) begin
            wleft  <= wleft - 1;
            wleft1 <= wleft == 2;
            if (wleft1) wstop <= 1;             // this was the last one
        end
        if (tick && capturing) begin
            if (rl_first || rl_end) begin rl_val <= samp[7:0]; rl_cnt <= 0; rl_full <= 0; rl_first <= 0; end
            else begin rl_cnt <= rl_cnt + 1; rl_full <= rl_cnt == 8'hFE; end
        end
        if (we && state == ARMED && filled != DEPTH) filled <= filled + 1;
        pre_ok <= filled >= pre_need;                   // one cycle behind; cleared on arm below

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
                ARMED: if (tick && trig && pre_ok) begin
                    wleft  <= (dcount + 1) * 4;
                    wleft1 <= 0;
                    n     <= (dcount + 1) * 4;  // loads are always >= 4, so n_one goes low
                    n_one <= 0;
                    state <= POST;
                end
                POST: if (wp_r) begin           // counts entries written after the trigger: samples, or runs in RLE mode
                    n     <= n - 1;
                    n_one <= n == 2;
                    if (n_one) begin
                        ra    <= wa_r;          // the last counted entry; any later write goes past it
                        n     <= (rcount + 1) * 4;
                        n_one <= 0;
                        state <= LOAD0;
                    end
                end
                LOAD0: state <= LOAD;           // the newest entry's write lands this cycle
                LIVE: if (tx_ok && lv_avail && !lv_pop && steady) begin   // each record: high byte, then low
                    tx_data  <= out_b;
                    tx_start <= 1;
                    lv_second <= !lv_second;
                    lv_pop   <= lv_second;
                end
                LOAD: begin                     // wait out BRAM read latency
                    hi_byte <= !groups[0];      // group 0 disabled: start with group 1
                    state   <= SEND;
                end
                SEND: if (tx_ok && steady) begin   // SUMP sends newest sample first, enabled groups low first
                    tx_data  <= out_b;
                    tx_start <= 1;
                    if (!hi_byte && groups[1]) hi_byte <= 1;
                    else begin
                        ra    <= ra - 1;
                        n     <= n - 1;
                        n_one <= n == 2;
                        state <= n_one ? IDLE : LOAD;
                    end
                end
            endcase

            // Commands: bit 7 clear = 1 byte; set = 1 byte + 4 little-endian arg bytes.
            if (do_long) case (cmd)
                8'h80: div <= arg[23:0];
                8'h82: begin                    // flags: demux, and channel groups 0/1 disabled
                    demux  <= arg[0];
                    rle    <= arg[14];
                    groups <= (~arg[3:2] == 2'b00) ? 2'b01 : ~arg[3:2];   // never send nothing
                end
                8'h81: {dcount, rcount} <= arg;
                8'hC0: tmask <= arg[15:0];
                8'hC1: tval  <= arg[15:0];
                8'h90: {lcpb0, len0, lch0} <= {arg[31:12], arg[4], arg[3:0]};   // live receiver 0
                8'h91: {lcpb1, len1, lch1} <= {arg[31:12], arg[4], arg[3:0]};   // live receiver 1
                default: ;                      // other flags, other trigger stages: ignored
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
                    8'h01: begin state <= ARMED; tick <= 1; filled <= 0; pre_ok <= 0; rl_first <= 1; wstop <= 0; end  // run
                    8'h02: begin state <= ROM; rp <= 0; rend <= 3;  end     // ID
                    8'h04: begin state <= ROM; rp <= 4; rend <= 27; end     // metadata
                    8'h03: begin state <= LIVE; lv_second <= 0; end        // live UART log
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

// Live UART logging. Two receivers turn bytes into 2-byte records in a FIFO the host drains:
//   data     1 r e t4..t0 | byte    receiver r, framing error e, time since the previous record
//   time     0 t14..t8    | t7..t0  for gaps too long for a data record (never 0x7F in the high byte)
//   dropped  0x7F         | count   the FIFO was full: this many bytes were lost
// Time counts 10 us boundaries crossed, so deltas add up without drift. run low clears everything.
module live_log #(parameter LW = 11, parameter UNIT = 2000) (   // UNIT: 10 us of 200 MHz
    input clk, run,
    input [15:0] in,
    input [3:0] ch0, ch1,
    input en0, en1,
    input [19:0] cpb0, cpb1,
    input pop,                  // the host took the front record
    output [15:0] front,
    output reg avail = 0
);
    reg l0 = 1, l1 = 1;
    always @(posedge clk) begin l0 <= in[ch0] | !en0; l1 <= in[ch1] | !en1; end
    wire v0, v1, e0, e1;
    wire [7:0] d0, d1;
    uart_sniff rx0 (clk, run & en0, l0, cpb0, v0, d0, e0);
    uart_sniff rx1 (clk, run & en1, l1, cpb1, v1, d1, e1);

    // FIFO
    localparam DEPTH = 1 << LW;
    reg [15:0] fifo [0:DEPTH-1];
    reg [LW-1:0] wp = 0, rp = 0;
    reg [LW:0] cnt = 0;
    reg [15:0] q;
    reg push = 0;
    reg [15:0] pdata = 0;
    reg full = 0;                               // registered for 200 MHz: 2 slots early covers its lag
    always @(posedge clk) begin
        if (push) fifo[wp] <= pdata;
        q <= fifo[rp];
    end
    assign front = q;

    // records
    reg [10:0] unit = 0;
    reg [14:0] ticks = 0;                       // 10 us steps since the last record
    reg p0 = 0, p1 = 0, f0 = 0, f1 = 0;         // a received byte waiting to be written
    reg [7:0] b0 = 0, b1 = 0, dropped = 0;
    wire both_busy = p0 && p1;
    always @(posedge clk) begin
        push  <= 0;
        avail <= cnt != 0 && !push && !pop && run;
        full  <= cnt >= DEPTH - 2;   // registered for 200 MHz; q is a cycle behind any push or pop
        if (!run) begin
            wp <= 0; rp <= 0; cnt <= 0; unit <= 0; ticks <= 0; p0 <= 0; p1 <= 0; dropped <= 0;
        end else begin
            if (unit == UNIT - 1) begin
                unit <= 0;
                if (ticks != 15'h7EFF) ticks <= ticks + 1;
            end else unit <= unit + 1;
            if (v0) begin if (p0 && dropped != 8'hFF) dropped <= dropped + 1; p0 <= 1; b0 <= d0; f0 <= e0; end
            if (v1) begin if (p1 && dropped != 8'hFF) dropped <= dropped + 1; p1 <= 1; b1 <= d1; f1 <= e1; end

            if (push) begin wp <= wp + 1; end
            if (pop)  begin rp <= rp + 1; end
            cnt <= cnt + push - pop;

            if (!push) begin                    // at most one record per two cycles: plenty, bytes are microseconds apart
                if (dropped != 0 && !full) begin
                    push <= 1; pdata <= {8'h7F, dropped}; dropped <= 0;
                end else if (p0 || p1) begin
                    if (ticks > 31) begin
                        if (!full) begin push <= 1; pdata <= {1'b0, ticks}; ticks <= 0; end
                    end else if (full) begin    // no room: count it, keep going
                        if (dropped != 8'hFF) dropped <= dropped + 1;
                        if (p0) p0 <= 0; else p1 <= 0;
                    end else begin
                        push  <= 1;
                        pdata <= p0 ? {1'b1, 1'b0, f0, ticks[4:0], b0} : {1'b1, 1'b1, f1, ticks[4:0], b1};
                        ticks <= 0;
                        if (p0) p0 <= 0; else p1 <= 0;
                    end
                end else if (ticks == 15'h7EFF && !full) begin   // a long silence: mark the time anyway
                    push <= 1; pdata <= {1'b0, ticks}; ticks <= 0;
                end
            end
        end
    end
endmodule

// 8N1 UART receiver with a run-time bit time (clocks per bit), for live logging. rx is synchronized.
module uart_sniff (
    input clk, run, rx,
    input [19:0] cpb,
    output reg valid = 0,
    output reg [7:0] data = 0,
    output reg ferr = 0
);
    reg busy = 0, prev = 1;
    reg [19:0] cnt = 0;
    reg [3:0] bitn = 0;
    always @(posedge clk) begin
        valid <= 0;
        prev  <= rx;
        if (!run) busy <= 0;
        else if (!busy) begin
            if (prev && !rx) begin busy <= 1; cnt <= cpb >> 1; bitn <= 0; end   // start bit: wait half a bit
        end else if (cnt != 0) cnt <= cnt - 1;
        else begin
            cnt  <= cpb - 1;
            bitn <= bitn + 1;
            if (bitn == 0) begin if (rx) busy <= 0; end                      // glitch, not a start bit
            else if (bitn <= 8) data <= {rx, data[7:1]};
            else begin busy <= 0; valid <= 1; ferr <= !rx; end              // stop bit
        end
    end
endmodule
