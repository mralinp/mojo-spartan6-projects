// UART traffic for the Mojo logic analyzer, from an Arduino Pro Micro (ATmega32U4).
//
// Serial1 is the hardware UART on the TXO and RXI pins, wired to a USB-TTL dongle.
//   TXO: "tick N" every TICK_MS, so the analyzer always has something to catch on D0
//   RXI: every line you type into the dongle's terminal comes back as "echo: <line>"
//
// Probe TXO with D0 and RXI with D1. The Mojo's inputs take 3.3 V only: use the
// 3.3 V / 8 MHz Pro Micro, or put a divider on each probe line with a 5 V one.

const unsigned long BAUD = 115200;  // match the dongle's terminal; Mojo LA detects it by itself
const unsigned long TICK_MS = 100;

unsigned long tick = 0, lastTick = 0;
char line[64];
size_t len = 0;

void setup() {
  Serial1.begin(BAUD);
}

void loop() {
  if (millis() - lastTick >= TICK_MS) {
    lastTick = millis();
    Serial1.print(F("tick "));
    Serial1.println(tick++);
  }

  while (Serial1.available()) {
    char c = Serial1.read();
    if (c == '\r' || c == '\n') {
      if (len) {
        line[len] = '\0';
        Serial1.print(F("echo: "));
        Serial1.println(line);
        len = 0;
      }
    } else if (len < sizeof line - 1) {
      line[len++] = c;
    }
  }
}
