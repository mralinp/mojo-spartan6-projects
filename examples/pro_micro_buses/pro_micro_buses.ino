// UART, I2C and SPI at once for the Mojo logic analyzer, from an Arduino Pro Micro (ATmega32U4).
//
// Every 200 ms, back to back, so one 16 ms capture at 1 MHz catches all three:
//   UART  Serial1 TXO "tick N"; lines received on RXI are echoed      -> Mojo D0 (TXO), D1 (RXI)
//   I2C   bit-banged on pin 3 (SCL) and pin 2 (SDA), ~50 kHz          -> Mojo D2 (SCL), D3 (SDA)
//   SPI   hardware, mode 0, 250 kHz: 15 SCK, 16 MOSI, 14 MISO, 10 CS  -> Mojo D4, D5, D6, D7
//
// There's no real I2C device on the bus, so the sketch plays both sides: it drives the ACKs and
// the bytes a device would return. It switches an SSD1306 display on at 0x3C, then reads a TMP102
// at 0x48 as 25.5 C. SPI sends a flash JEDEC-ID read (9F 00 00 00); with nothing attached, MISO
// reads FF through its pull-up.
//
// The Mojo takes 3.3 V only: use the 3.3 V / 8 MHz Pro Micro, or a divider on every probe line.

#include <SPI.h>

const unsigned long BAUD = 115200;
const unsigned long BURST_MS = 200;
const uint8_t SCL_PIN = 3, SDA_PIN = 2, CS_PIN = 10, MISO_PIN = 14;
const unsigned I2C_HALF_US = 5;  // half an I2C clock, plus digitalWrite's own time

// ---- I2C, open drain: drive low or let the pull-up take the line high ----
void line(uint8_t pin, bool high) {
  if (high) {
    pinMode(pin, INPUT_PULLUP);
  } else {
    digitalWrite(pin, LOW);
    pinMode(pin, OUTPUT);
  }
}
void half() { delayMicroseconds(I2C_HALF_US); }
void i2cStart() {  // also a repeated start: SCL is low after the last bit
  line(SDA_PIN, true); line(SCL_PIN, true); half();
  line(SDA_PIN, false); half();
  line(SCL_PIN, false); half();
}
void i2cStop() {
  line(SDA_PIN, false); half();
  line(SCL_PIN, true); half();
  line(SDA_PIN, true); half();
}
void i2cBit(bool b) {
  line(SDA_PIN, b); half();
  line(SCL_PIN, true); half();
  line(SCL_PIN, false);
}
void i2cByte(uint8_t v, bool ack) {
  for (int8_t k = 7; k >= 0; k--) i2cBit((v >> k) & 1);
  i2cBit(!ack);  // 9th clock: SDA low is ACK
}

// ---- UART echo ----
char lineBuf[64];
size_t len = 0;
unsigned long tick = 0, last = 0;

void setup() {
  Serial1.begin(BAUD);
  line(SCL_PIN, true);
  line(SDA_PIN, true);
  SPI.begin();
  pinMode(CS_PIN, OUTPUT);
  digitalWrite(CS_PIN, HIGH);
  pinMode(MISO_PIN, INPUT_PULLUP);
}

void loop() {
  if (millis() - last >= BURST_MS) {
    last = millis();
    Serial1.print(F("tick "));  // goes out by interrupt while the I2C and SPI below run
    Serial1.println(tick++);

    i2cStart();                  // SSD1306: command stream, display on
    i2cByte(0x3C << 1, true);
    i2cByte(0x00, true);
    i2cByte(0xAF, true);
    i2cStop();

    i2cStart();                  // TMP102: point at the temperature register, read it back
    i2cByte(0x48 << 1, true);
    i2cByte(0x00, true);
    i2cStart();
    i2cByte(0x48 << 1 | 1, true);
    i2cByte(0x19, true);         // 0x1980 >> 4 = 408 * 0.0625 = 25.5 C
    i2cByte(0x80, false);        // the master NACKs the last byte it wants
    i2cStop();

    SPI.beginTransaction(SPISettings(250000, MSBFIRST, SPI_MODE0));
    digitalWrite(CS_PIN, LOW);
    SPI.transfer(0x9F);          // JEDEC ID
    for (uint8_t k = 0; k < 3; k++) SPI.transfer(0x00);
    digitalWrite(CS_PIN, HIGH);
    SPI.endTransaction();
  }

  while (Serial1.available()) {
    char c = Serial1.read();
    if (c == '\r' || c == '\n') {
      if (len) {
        lineBuf[len] = '\0';
        Serial1.print(F("echo: "));
        Serial1.println(lineBuf);
        len = 0;
      }
    } else if (len < sizeof lineBuf - 1) {
      lineBuf[len++] = c;
    }
  }
}
