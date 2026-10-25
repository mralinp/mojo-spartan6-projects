<p align="center">
  <img src="assets/logo.svg" alt="Mojo Spartan-6 Projects logo: a waveform M cut out of an FPGA chip" width="128">
</p>

<h1 align="center">Mojo Spartan-6 Projects</h1>

<p align="center">
  Verilog projects for the <b>Embedded Micro Mojo V3</b> FPGA board (Xilinx Spartan-6 XC6SLX9),<br>
  built and flashed from a Mac with no Windows or Linux machine in sight.
</p>

<p align="center">
  <img alt="FPGA" src="https://img.shields.io/badge/FPGA-Spartan--6%20XC6SLX9-c41e3a">
  <img alt="Language" src="https://img.shields.io/badge/HDL-Verilog-1f6feb">
  <img alt="Toolchain" src="https://img.shields.io/badge/ISE%2014.7-in%20Docker-2496ed">
  <img alt="Host" src="https://img.shields.io/badge/host-macOS%20(Apple%20silicon)-555">
</p>

---

## Projects

| Project | What it does | Resources |
|---|---|---|
| [**logic_analyzer**](logic_analyzer/) | 8-channel, 50 MHz, 16K-sample logic analyzer. Speaks the SUMP protocol, so PulseView / sigrok is the UI. | 7% slices, 8 of 32 BRAMs |
| [**led_counter**](led_counter/) | 8-bit binary counter on the onboard LEDs. The "hello world" that proves the toolchain works. | 1% slices |

## Quick start

```bash
tools/ise.sh build logic_analyzer                                   # Verilog -> bitstream (~3 min)
.venv/bin/python tools/mojo_load.py logic_analyzer/build/mojo_top.bin   # flash over USB (~13 s)
```

First time on a new Mac? See [Toolchain setup](#toolchain-setup) below.

## Logic analyzer

<table>
<tr><td>

| | |
|---|---|
| Channels | 8 |
| Max sample rate | 50 MHz |
| Memory | 16,384 samples |
| Trigger | pattern match (mask + value), pre/post-trigger split |
| Protocol | SUMP / Openbench Logic Sniffer |
| Link | USB through the Mojo's AVR, 500 kbaud |

</td><td>

| Channel | Pin |
|---|---|
| 0 | P51 |
| 1 | P50 |
| 2 | P41 |
| 3 | P40 |
| 4 | P35 |
| 5 | P34 |
| 6 | P33 |
| 7 | P32 |

</td></tr>
</table>

> [!WARNING]
> Inputs are **3.3 V LVTTL and not 5 V tolerant**. Put a level shifter or divider in front of 5 V logic.

**Capture with PulseView:** pick the driver *Openbench Logic Sniffer & SUMP compatibles*, serial port
`/dev/cu.usbmodem*`, then *Scan for devices*. It shows up as **Mojo LA** with 8 channels. The 8 LEDs mirror the
probe inputs live, which helps when wiring up.

**From the command line:**

```bash
sigrok-cli -d ols:conn=/dev/cu.usbmodem1201 --config samplerate=1m --samples 16k -o capture.sr
```

**Simulate** (needs `brew install icarus-verilog`): the testbench plays the host, checks the ID reply and a triggered
capture.

```bash
cd logic_analyzer && iverilog -o /tmp/tb tb.v mojo_top.v && vvp /tmp/tb
```

## Toolchain setup

Spartan-6 is only supported by **Xilinx ISE 14.7**, which never shipped for macOS. Here it runs in an x86 Linux
container (emulated on Apple silicon), and a small Python loader replaces the Mojo IDE for flashing.

1. **Docker image** with ISE's runtime libraries:
   ```bash
   tools/ise.sh image
   ```
2. **ISE 14.7 WebPACK.** Download *Full Installer for Linux* (`Xilinx_ISE_DS_Lin_14.7_1015_1.tar`, 6.5 GB) from
   AMD, then install it into a Docker volume. The installer shows Xilinx's EULA for you to accept.
   ```bash
   tools/ise.sh install ~/Downloads/Xilinx_ISE_DS_Lin_14.7_1015_1.tar
   ```
3. **License.** Generate a free *ISE WebPACK* license on AMD's licensing site and save it as `tools/Xilinx.lic`
   (it's git-ignored). If your license is node-locked, use host ID `020000001407`: the container always runs with
   that MAC address.
4. **Loader:**
   ```bash
   python3 -m venv .venv && .venv/bin/pip install pyserial
   ```

## Things the Mojo taught me

Two quirks that cost real debugging time, written down so they don't cost it again:

- **Large USB writes wedge the AVR on modern macOS.** The bootloader acknowledges the command and size, then
  stops reading mid-transfer until the board is power-cycled. Writing the bitstream in 32-byte pieces fixes it, so
  `mojo_load.py` does exactly that.
- **`avr_rx_busy` is a toggle, not a level.** The AVR firmware flips the line once per byte it has room for and
  freezes it when its buffer is nearly full. Treating it as a level deadlocks the FPGA; ignoring it drops a third of
  a 16K capture. The analyzer sends one byte per edge, and a full 16K capture now arrives intact in about a second.

## Layout

```
assets/logo.svg   the logo: a waveform M cut out of an FPGA chip
logic_analyzer/   mojo_top.v (design + UARTs), mojo.ucf (pins), tb.v (testbench)
led_counter/      mojo_top.v, mojo.ucf
tools/
  ise.sh          image / install / build, all through Docker
  Dockerfile      ISE runtime libraries (Ubuntu 18.04, linux/amd64)
  mojo_load.py    USB flasher, same protocol as Embedded Micro's Mojo Loader
```

Each project is one folder with a `mojo_top.v` and a `mojo.ucf`. Copy `led_counter/` to start a new one.
