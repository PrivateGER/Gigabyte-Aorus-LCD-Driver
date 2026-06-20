# Gigabyte GPU LCD driver

Rust binary that can control the LCD on the Gigabyte GeForce RTX 5080 AORUS
Master ICE. It may work with other Gigabyte LCD screens, but they can use
different I2C addresses, device ids, storage offsets, or timing requirements.

<center><img width="1024" height="771" alt="blahaj with GPU stats" src="https://github.com/user-attachments/assets/b2e429de-2743-4591-8662-fecd2c4954f9" /></center>

GIF support is included, but the panel _really_ sucks at handling them. 
They are very panel-sensitive and can get stuck on Loading if the file is too complex, or the panel simply doesn't feel like it.

In some games, in some cases, the metric display switching seems to cause micro-stutters (~20ms spikes) whenever the display updates. I sometimes experience the same with the Windows driver, so I wonder if this is just fundamental to how the card LCD works. 

It does *not* occur when using a static image. More data needed, if you can tell me about your experience that'd be very helpful.

## Requirements

- Linux with access to the GPU LCD I2C adapter.
- Permission to open `/dev/nvidiactl` and `/dev/nvidia*` (default transport),
  or `/dev/i2c-1` for the fallback transport.
- NVIDIA NVML available from the installed driver.
- Rust toolchain if building from source.

## I2C Transport And Frametime Hitches

Going through `/dev/i2c-N` clocks the GPU I2C bus at a fixed 100 kHz and the
NVIDIA driver holds its global GPU locks for the whole transfer, so every
256-byte panel page stalls frame presentation for ~27 ms, causing a visible hitch
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

On top of the faster bus, the service skips metric writes entirely when no
displayed value has changed beyond display noise (2 °C, 2 pp usage, 3 W,
15 MHz, 50 RPM) relative to the last written sample, which the Windows
client does not do. Stable values are force-refreshed every 30 s as a
staleness guard. `--update-interval SEC` (default 1) stretches the
check cadence further if desired.

There *is still a hitch* with all these tweaks, but this is also the case on Windows and lasts <8ms per.

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

## Run With Nix

Build or run the flake package directly:

```bash
nix build github:PrivateGER/Gigabyte-Aorus-LCD-Driver
nix run github:PrivateGER/Gigabyte-Aorus-LCD-Driver -- --help
```

## Home Manager Service

The flake exposes a reusable Home Manager module:

```nix
{
  inputs.gigabyte-lcd.url = "github:PrivateGER/Gigabyte-Aorus-LCD-Driver";

  outputs = { home-manager, nixpkgs, gigabyte-lcd, ... }: {
    homeConfigurations.example = home-manager.lib.homeManagerConfiguration {
      pkgs = import nixpkgs { system = "x86_64-linux"; };
      modules = [
        gigabyte-lcd.homeModules.default
        {
          services.gigabyte-lcd = {
            enable = true;
            mascot = "/home/example/.config/gigabyte-lcd/background.png";
            bus = 1;
            addr = "0x61";
            deviceId = "0x21";
            imageSettleDelay = 20;
            metrics = [
              "temp"
              "usage"
              "power"
            ];
            overlayInterval = 4;
          };
        }
      ];
    };
  };
}
```

By default, enabling the module also enables the user systemd service for
`default.target`. Set `services.gigabyte-lcd.systemdTargets = [ ];` to install
the unit without starting it automatically.

## Experimental GIF Mode

GIF mode can be tested:

```bash
target/release/gigabyte-lcd \
  --mascot blahaj.png \
  --gif blahaj.gif
```

This is VERY unstable! Some gifs may work, some may not. Set the static image mode again to clear a stuck screen.
