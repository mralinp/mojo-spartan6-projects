# Runtime libs for ISE 14.7 command-line tools. ISE itself lives in the
# "ise147" Docker volume (installed once by `tools/ise.sh install`).
FROM ubuntu:18.04
RUN apt-get update && apt-get install -y --no-install-recommends \
        libncurses5 libglib2.0-0 libsm6 libxi6 libxrender1 libxrandr2 \
        libxext6 libxtst6 libfreetype6 libfontconfig1 libusb-0.1-4 make \
    && rm -rf /var/lib/apt/lists/*
