# Gigabyte GPU LCD

Rust binary that can control the LCD on the Gigabyte GeForce RTX 5080 AORUS
Master ICE. It may work with other Gigabyte LCD screens, but they can use
different I2C addresses, device ids, storage offsets, or timing requirements.

GIF support is included, but the panel _really_ sucks at handling them. 
They are very panel-sensitive and can get stuck on Loading if the file is too complex, or the panel simply doesn't feel like it.

## Requirements

- Linux with access to the GPU LCD I2C adapter.
- Permission to open `/dev/nvidiactl` and `/dev/nvidia*` (default transport),
  or `/dev/i2c-1` for the fallback transport.
- NVIDIA NVML available from the installed driver.
- Rust toolchain if building from source.

## I2C Transport And Frametime Hitches

Going through `/dev/i2c-N` clocks the GPU I2C bus at a fixed 100 kHz and the
NVIDIA driver holds its global GPU locks for the whole transfer, so every
256-byte panel page stalls frame presentation for ~27 ms — a visible hitch
once per second while the metric overlay updates. The Windows driver avoids
this by requesting 400 kHz per transaction through NVAPI.

This service does the same on Linux: by default it talks to the NVIDIA
resource manager directly (`/dev/nvidiactl`) and issues I2C transactions at
400 kHz, cutting the stall to roughly a quarter. Control it with:

```text
--transport auto|rm|i2c-dev       default: auto (RM, falling back to i2c-dev)
--i2c-speed-khz 100|200|300|400   default: 400 (RM transport only)
```

The RM transport resolves the GPU and port automatically from the `--bus`
number via the sysfs adapter name, so multi-GPU systems keep working.

Tested hardware:

```text
GPU: Gigabyte GeForce RTX 5080 AORUS Master ICE 16GB
LCD: 320 x 170
I2C bus: /dev/i2c-1
I2C address: 0x61
Device id: 0x21
```

## Finding Your LCD Bus And IDs

This oneliner checks your i2c devices and tries to find the LCD address. No need to run this if you have the same card.

This *may* cause a card reset, don't be alarmed. The card will restart and you'll get a proper result.
```bash
for b in $(i2cdetect -l | awk '/NVIDIA|nvkm|nouveau/ {sub("i2c-","",$1); print $1}'); do for a in $(i2cdetect -y "$b" | awk 'NR>1{for(i=2;i<=NF;i++) if($i!="--" && $i!="UU" && $i!="50") print "0x"$i}'); do fw=$(i2ctransfer -y "$b" w256@"$a" 0xd6 0xcb 0x55 0xac 0x38 0= r4 2>/dev/null | tr -d '\n'); [ "$fw" = "0xd6 0x14 0x01 0x02" ] && echo "LCD: --bus $b --addr $a --device-id 0x21 (firmware F1.4)"; done; done
```

## Run Static Mode

```bash
target/release/gigabyte-lcd \
  --mascot blahaj.png \
  --metrics temp,usage,power \
  --overlay-interval 4
```

Select overlay fields with `--metrics`. Supported names are:

```text
temp, clock, usage, fan, vram-clock, vram, power, all, none
```

The default is `temp,usage,power`.

After setup, the service updates values once per
second.

To upload the selected image or GIF and exit without starting NVML metric
monitoring, add `--no-monitoring`:

```bash
target/release/gigabyte-lcd \
  --mascot blahaj.png \
  --gif blahaj.gif \
  --no-monitoring
```

## Install User Service

Build the release binary first, install the binary and background image into
stable user paths, then install the included user unit:

```bash
cargo build --release
install -Dm755 target/release/gigabyte-lcd ~/.local/bin/gigabyte-lcd
install -Dm644 path/to/background.png ~/.config/gigabyte-lcd/background.png
mkdir -p ~/.config/systemd/user
cp systemd/gigabyte-lcd-rust.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now gigabyte-lcd-rust.service
```

Edit `systemd/gigabyte-lcd-rust.service` with your own image.

## Experimental GIF Mode

GIF mode can be tested:

```bash
target/release/gigabyte-lcd \
  --mascot blahaj.png \
  --gif blahaj.gif
```

This is VERY unstable! Some gifs may work, some may not. Set the static image mode again to clear a stuck screen.
