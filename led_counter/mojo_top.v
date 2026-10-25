// 8-bit binary counter on the Mojo V3 LEDs, ~3 counts per second.
module mojo_top (
    input        clk,          // 50 MHz
    input        rst_n,        // reset button
    output [7:0] led,
    output       spi_miso,     // pins shared with the AVR: leave floating
    output [3:0] spi_channel,
    output       avr_rx
);
    reg [31:0] cnt = 0;
    always @(posedge clk)
        if (!rst_n) cnt <= 0;
        else cnt <= cnt + 1;

    assign led         = cnt[31:24];
    assign spi_miso    = 1'bz;
    assign spi_channel = 4'bzzzz;
    assign avr_rx      = 1'bz;
endmodule
