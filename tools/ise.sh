#!/bin/sh
# ISE 14.7 in Docker (linux/amd64) for the Mojo V3.
#   tools/ise.sh image                   build the runtime image
#   tools/ise.sh install <ISE .tar>      install ISE WebPACK into the ise147 volume (interactive EULA)
#   tools/ise.sh build <project dir>     Verilog + mojo.ucf -> <dir>/build/mojo_top.bin
set -e
IMG=mojo-ise
ROOT=$(cd "$(dirname "$0")/.." && pwd)
# ISE's free WebPACK license is tied to a MAC address; keep the container's fixed.
RUN="docker run --rm --platform linux/amd64 --mac-address 02:00:00:00:14:07 -v ise147:/opt/Xilinx"
PART=xc6slx9-2-tqg144

case "$1" in
image)
    docker build --platform linux/amd64 -t $IMG "$ROOT/tools"
    ;;
install)
    tar=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
    $RUN -it -e TERM=xterm -v "$tar":/ise.tar:ro $IMG bash -c "
        set -e
        mkdir -p /tmp/ise && tar -xf /ise.tar -C /tmp/ise && cd /tmp/ise/*/
        printf '%s\n' 'destination_dir=/opt/Xilinx' 'copy_preferences=N' 'use_multiple_cores=Y' \
            'application=Acquire or Manage a License Key::0' 'package=ISE WebPACK::1' \
            'application=setupEnv.sh::0' 'application=Install Linux System Generator Info XML::0' \
            'application=Ensure Linux System Generator Symlinks::0' 'application=Install Cable Drivers::0' > /tmp/webpack.txt
        ./bin/lin64/batchxsetup --batch /tmp/webpack.txt"
    ;;
build)
    dir=$(cd "$2" && pwd)
    lic=""
    [ -f "$ROOT/tools/Xilinx.lic" ] && lic="-v $ROOT/tools/Xilinx.lic:/root/.Xilinx/Xilinx.lic:ro"
    $RUN $lic -v "$dir":/src -w /src $IMG bash -c "
        set -e
        . /opt/Xilinx/14.7/ISE_DS/settings64.sh >/dev/null
        mkdir -p build && cd build
        ls ../*.v | grep -v '/tb' | sed 's|^|verilog work |' > top.prj
        echo 'run -ifn top.prj -ifmt mixed -ofn mojo_top -ofmt NGC -p $PART -top mojo_top -opt_mode Speed -opt_level 1' > top.xst
        xst -ifn top.xst -ofn xst.log > /dev/null || { tail -30 xst.log; exit 1; }
        ngdbuild -quiet -uc ../mojo.ucf -p $PART mojo_top.ngc mojo_top.ngd
        map -w -p $PART -o mojo_top_map.ncd mojo_top.ngd mojo_top.pcf > map.out
        par -w mojo_top_map.ncd mojo_top.ncd mojo_top.pcf > par.out
        grep -E 'All constraints were met|timing errors' mojo_top.par || true
        bitgen -w -g Binary:Yes mojo_top.ncd mojo_top.bit mojo_top.pcf > bitgen.out
        ls -l mojo_top.bin"
    ;;
*)
    sed -n '2,5p' "$0"; exit 1
    ;;
esac
