#!/usr/bin/env python3
"""Load a bitgen .bin onto a Mojo V3 over USB (same protocol as Mojo Loader).

usage: mojo_load.py [--ram] [--port /dev/cu.usbmodemXXXX] file.bin
Default writes SPI flash (survives power cycles); --ram is faster but temporary.
"""
import argparse, glob, sys, time
import serial


def expect(s, want, what, timeout=5):
    s.timeout = timeout
    got = s.read(1)
    if got != want:
        sys.exit(f"error: {what} (got {got!r}, want {want!r})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("bin")
    ap.add_argument("--ram", action="store_true")
    ap.add_argument("--port", default=(glob.glob("/dev/cu.usbmodem*") or [None])[0])
    a = ap.parse_args()
    if not a.port:
        sys.exit("error: no /dev/cu.usbmodem* port found, is the Mojo plugged in?")
    data = open(a.bin, "rb").read()

    with serial.Serial(a.port, 115200) as s:
        # Pulsing DTR resets the AVR into its loader.
        s.dtr = False
        time.sleep(0.005)
        for _ in range(5):
            s.dtr = False; time.sleep(0.005)
            s.dtr = True; time.sleep(0.005)
        s.reset_input_buffer()

        s.write(b"R" if a.ram else b"F")
        expect(s, b"R", "Mojo did not respond")
        s.write(len(data).to_bytes(4, "little"))
        expect(s, b"O", "Mojo did not accept the size")
        # Big writes wedge the AVR's USB stack on current macOS; sub-packet writes don't.
        for i in range(0, len(data), 32):
            s.write(data[i:i + 32])
            s.flush()
        expect(s, b"D", "Mojo did not acknowledge the transfer", timeout=60)
        if not a.ram:
            s.write(b"L")                      # configure FPGA from the new flash image
            expect(s, b"D", "Mojo could not load from flash", timeout=10)
    print(f"loaded {len(data)} bytes to {'RAM' if a.ram else 'flash'} via {a.port}")


if __name__ == "__main__":
    main()
