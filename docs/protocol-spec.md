# Gigabyte GPU LCD Protocol Specification

## 1. Scope

This contains the reverse-engineered control protocol for Gigabyte GPU LCD panels using the older Gigabyte Control Center LCD command format, extracted from a decompiled Gigabyte Control Center install.

The implementation targets this device profile specifically:

| Field | Value |
| --- | --- |
| GPU | Gigabyte GeForce RTX 5080 AORUS Master ICE 16GB |
| LCD size | 320 x 170 pixels |
| Linux I2C device | `/dev/i2c-1` |
| 7-bit I2C address | `0x61` |
| GCC write address equivalent | `0xc2` |
| GCC device id | `0x21` |
| Firmware readback | `F1.4` |

Other Gigabyte LCD panels/GPUs may share the protocol but use different I2C buses,
addresses, device ids, storage offsets, or timing requirements.

## 2. Terminology

Command packets are host-to-LCD I2C writes. Readback commands use an I2C
write-read transaction. Multi-byte integer byte order is specified per field.

## 3. Transport

### 3.1 I2C Framing

Command packets MUST be exactly 256 bytes.

All old-protocol command packets use this format:

```text
u8 opcode
u8[4] magic = cb 55 ac 38
u8[] args
u8[] zero_padding_to_256_bytes
```

Upload data pages are raw 256-byte writes and do not include the magic prefix.

### 3.2 Readback Limits

The protocol exposes firmware and mode readback. It does not expose a readback for image decode completion,
GIF decode completion, current visible frame, or panel Loading state.

## 4. Display Modes

| Mode id | Name | `0xe5` argument |
| --- | --- | --- |
| `0` | Faith1 | `1` |
| `1` | Faith2 | `2` |
| `2` | Faith3 | `3` |
| `3` | Static image | `4` |
| `4` | Vendor text slot | `5` |
| `5` | GIF/animation | `6` |
| `6` | Chibi/time | `7` |
| `7` | Carousel | `10` |

For all modes except carousel, the `0xe5` argument is `mode id + 1`. Carousel
uses wire mode `9`, therefore its command argument is `10`.

## 5. Commands

### 5.1 Command Summary

| Opcode | Name | Direction | Purpose                                                         |
| --- | --- | --- |-----------------------------------------------------------------|
| `0xd6` | Read firmware | write-read | Diagnostic read.                                                |
| `0xde` | Read mode | write-read | Diagnostic read of controller mode/enabled state.               |
| `0xe7` | Open LCD | write | Enables or disables the LCD controller.                         |
| `0xe5` | Set mode | write | Selects display mode.                                           |
| `0xe1` | Set metric overlay | write | Selects overlay fields and rotation interval.                   |
| `0xe3` | Set metric values | write | Refreshes cached metric values.                                 |
| `0xea` | Set image template | write | Configures template/color/positions for image-like modes.       |
| `0xf1` | Upload header | write | Declares payload target, pages, frame count, delay, chunk mode. |
| `0xf2 0x01` | Upload start | write | Starts an upload transaction.                                   |
| `0xf2 0x02` | Upload finish | write | Finishes an upload transaction.                                 |
| `0xf3` | Set loop list | write | Configures carousel/loop mode list.                             |
| `0xaa` | Save | write | Persists settings. Not used by default here.                    |

### 5.2 Open LCD: `0xe7`

Arguments:

| Byte | Meaning |
| --- | --- |
| `0x01` | Enable LCD |
| `0x02` | Disable LCD |

The service MUST enable the LCD before upload or mode setup.

### 5.3 Set Mode: `0xe5`

Arguments:

```text
u8 mode_argument
```

`mode_argument` is defined by the table in section 4.

### 5.4 Set Loop List: `0xf3`

Arguments:

```text
u8 interval
u8[] mode_arguments_without_carousel_special_case
```

The static service uses this command to lock the loop to image mode before
entering steady state.

### 5.5 Set Image Template: `0xea`

Arguments:

```text
u8 template_kind
u8 red
u8 green
u8 blue
u16 image_x_be
u16 image_y_be
u16 data_x_be
u16 data_y_be
u8 enabled
```

Template kinds:

| Value | Kind |
| --- | --- |
| `1` | GIF |
| `2` | Image |
| `3` | Pet/chibi |

Default implementation values:

```text
color: ff ff ff
image position: 0, 320
data position: 146, 64
enabled: 1
```

### 5.6 Set Metric Overlay: `0xe1`

Arguments:

```text
u8 gpu_temperature_enabled
u8 gpu_clock_enabled
u8 gpu_usage_enabled
u8 fan_speed_enabled
u8 vram_clock_enabled
u8 vram_usage_enabled
u8 fps_enabled
u8 power_tgp_enabled
u8 rotation_interval
```

Flag bit mapping used by the CLI:

| Bit | Metric |
| --- | --- |
| `0` | GPU temperature |
| `1` | GPU clock |
| `2` | GPU usage |
| `3` | Fan speed |
| `4` | VRAM clock |
| `5` | VRAM usage |
| `6` | FPS |
| `7` | TGP/power |

The release service selects:

```text
flags: 0x85
fields: GPU temperature, GPU usage, TGP/power
rotation interval: 4
```

`0xe1` is a setup command. You should NOT send it repeatedly, it has intensive performance issues.

### 5.7 Set Metric Values: `0xe3`

Arguments:

```text
u8 gpu_temperature_c
u16 gpu_clock_mhz_be
u8 gpu_usage_percent
u16 fan_rpm_be
u16 vram_clock_mhz_be
u8 vram_usage_percent
u16 fps_be
u16 power_watts_be
```

The LCD displays cached values from this packet. It does not poll GPU telemetry
on its own!

The release service sends `0xe3` once per second in steady state.

## 6. Upload Protocol

### 6.1 Upload Transaction

An upload transaction MUST use this sequence:

```text
sleep 2 seconds
send 0xf2 0x01
sleep 500 ms
send 0xf1 upload header
perform chunk preparation sleeps
sleep 1 second
send 256-byte payload pages, sleeping 1 ms after each page
sleep 500 ms
send 0xf2 0x02
```

### 6.2 Upload Header: `0xf1`

Arguments:

```text
u32 target_address_be
u8 storage_type
u32 page_count_be
u16 frame_count_be
u8 frame_delay_ms
u8 chunk_mode
u8 reserved_zero
```

Page count is:

```text
payload_size / 256 + 1
```

The extra page is included even when `payload_size` is an exact multiple of
256.

### 6.3 Chunk Modes

| Payload size | Chunk mode | Preparation unit | Delay per unit |
| --- | --- | --- | --- |
| `< 20480 bytes` | `1` | 4096 bytes | 400 ms |
| `>= 20480 bytes` | `2` | 65536 bytes | 2 s |

### 6.4 Storage Targets

For 50-series device ids `0x18`, `0x19`, `0x20`, `0x21`, and `0x22`:

| Payload kind | Target address | Storage type | Display mode |
| --- | --- | --- | --- |
| Static image | `0x01300000` | `1` | `3` |
| Text | `0x01320000` | `1` | `4` |
| GIF | `0x00000000` | `2` | `5` |

For older/non-50-series ids, static image uses `0x01f26000` and text uses
`0x01f00000`.

## 7. Payload Formats

### 7.1 Pixel Format

Image pixels are RGB565 little-endian.

Panel dimensions are:

```text
width: 320
height: 170
```

### 7.2 Static Image Container

Static image payload:

```text
u16_le frame_count = 1
u32_le frame_0_end_offset_minus_one
u16_le width
u16_le height
u16_le image_format = 1
u8[] rgb565_le_pixels
```

For a full 320 x 170 image, payload size is 108812 bytes.

### 7.3 Animation Container

Animation/GIF payload:

```text
u16_le frame_count
frame_header[frame_count]
u8[] rle_frame_streams
```

Each frame header is:

```text
u32_le frame_end_offset_minus_one
u16_le width
u16_le height
u16_le image_format = 3
```

Frame streams are RGB565 RLE streams.

### 7.4 RLE Stream

RLE is encoded as 16-bit little-endian words.

Literal block:

```text
u16_le count, high bit clear
u16_le[count] literal_rgb565_values
```

Repeat block:

```text
u16_le count_with_high_bit_set
u16_le repeated_rgb565_value
```

The repeat count is `count_with_high_bit_set & 0x7fff`. Literal and repeat
blocks MUST NOT encode more than 32767 pixels.

## 8. Supported States

### 8.1 Static Service State

This is the GCC stable release state machine:

```text
Disabled/unknown
  -> enable LCD
  -> upload static image payload
  -> wait 20 seconds
  -> clear metric overlay
  -> lock loop to image mode
  -> set image mode
  -> set image mode again after 300 ms
  -> enable image template
  -> set metric overlay flags 0x85 interval 4
  -> steady metric refresh
```

In steady metric refresh, the service MUST send only `0xe3` value packets at
the configured telemetry interval.

The service MUST NOT perform image uploads, mode changes, template writes,
save commands, or overlay-selection writes from the steady-state loop.

### 8.2 Experimental GIF State

GIF mode is supported for manual testing only. It is not the release service
default.

The current safe GIF startup profile is:

```text
upload static reset image first
apply image mode/template after reset
set GIF mode before GIF upload
enable GIF template before GIF upload
normalized content height <= 150 px
frame delay >= 100 ms
frame budget <= 24 source frames
post-GIF-upload delay = 5 seconds
```

The tested GIF payload was:

```text
62410 bytes
20 frames
100 ms frame delay
```

The GIF state machine is:

```text
Disabled/unknown
  -> enable LCD
  -> upload static reset image
  -> apply image mode/template
  -> clear metric overlay
  -> set GIF mode
  -> enable GIF template
  -> upload GIF payload
  -> wait 5 seconds
  -> clear metric overlay
  -> set GIF mode
  -> set metric overlay flags 0x85 interval 4
  -> steady metric refresh
```

GIF decode success cannot be verified automatically through known readbacks.

### 8.3 Recovery State

If the panel is white, stuck on Loading, or visibly refreshing incorrectly,
recover through the static service state:

```text
stop experimental/test service
start static image service
allow 20 second static settle
enter steady metric refresh
```

## 9. Performance Requirements

The host SHOULD use in-process NVML for telemetry. It SHOULD NOT invoke
`nvidia-smi` once per refresh.

The steady-state refresh loop MUST avoid repeated setup commands. In testing,
repeated `0xe1` overlay-selection writes and repeated mode/setup writes caused
visible frame-time hitches while games were running.

All LCD I2C writes SHOULD be serialized through a single writer.

## 10. GCC Ex and RGB Protocol Notes

Gigabyte Control Center contains newer Ex paths in addition to the old LCD
protocol implemented by this project. The Ex paths are documented here as
reverse-engineering notes; they are not yet proven as working Linux transports.

### 10.1 Transport Split

The managed `ucVga.dll` code has three relevant I2C wrappers:

| Wrapper | GCC use | Buffer size | GCC/native address field | Linux 7-bit equivalent |
| --- | --- | --- | --- | --- |
| `I2CApi` | Old LCD protocol | 256 bytes | Caller supplies shifted `0xc2` | `0x61`, proven |
| `I2CApi4LcdEx` | LCD Ex protocol | 256 bytes | Caller supplies `0x76`, wrapper shifts to `0xec` | `0x76`, unproven |
| `I2CApiEx` | RGB Ex protocol | 64 bytes | Wrapper ignores caller and hardcodes `0xea` | likely `0x75`, unproven |

All three wrappers set DDC port `1`, use no register-address phase
(`nRegAddrUsed = 0`), copy the command bytes into `szRegAddr`, and report native
success when `GvWriteI2C`/`GvReadI2C` returns `0`.

`I2CApi4LcdEx` retries writes/reads up to three times at speed `100`.
`I2CApiEx` retries writes/reads up to two times; RGB writes use speed `50`,
while RGB reads use speed `100`.

The current GCC log for this card showed `GvLcdExApi.GetFWVersion` failing, then
GCC falling back to the old LCD path. The old `0x61` LCD protocol therefore
remains the only proven LCD transport for this hardware.

### 10.2 LCD Ex Commands

LCD Ex commands use 256-byte packets with this shape:

```text
u8 opcode
u8 subcommand = 0x01 or transfer segment
u8[] args
u8[] zero_padding_to_256_bytes
```

Known LCD Ex commands:

| Opcode | Packet | Meaning |
| --- | --- | --- |
| `0x10` | `10 01` read 4 bytes | Firmware/version read, version is response bytes `2.3` |
| `0x12` | `12 01 effect speed bright r g b 00 area` | LCD-side LED area set |
| `0x13` | `13 01` | LCD-side LED save |
| `0x14` | `14 01 target` | Reset, `target=1` LED or `2` LCD |
| `0x15` | `15 01 state` | Open LCD, `state=1` enable or `2` disable |
| `0x16` | `16 01 mode` | Set LCD mode |
| `0x17` | `17 01 open [display interval r g b]` | Set display overlay open/closed and overlay attributes |
| `0x18` | `18 01 mode x_percent y_percent` | Set status overlay position |
| `0x19` | `19 01` | Set PC-power-off mode |
| `0x21` | segmented 140-byte packets | Upload LCD data |
| `0x22` | `22 01 count interval [mode tag]...` | Set loop/carousel list |
| `0x24` | `24 01` | Save LCD settings |

`0x17` first sends only the open flag. If open is `1`, GCC sends the same packet
again with display, interval, and RGB bytes populated. For LCD modes other than
`0`, `1`, and `2`, GCC also sends `0x18` with positions clamped to `0..100`
after multiplying the stored fractional positions by `100`.

`0x21` uploads data without the old `0xf1`/`0xf2` transaction wrapper. GCC splits
the payload into 256-byte blocks, then sends two 128-byte segments for each
block:

```text
u8 opcode = 0x21
u8 segment = 1 for first 128 bytes, 2 for second 128 bytes
u8 upload_mode
u8 file_picture_count
u16 block_index_le
u16 block_count_le
u8 segment_payload_len = 0x80
u8[3] reserved_zero
u8[128] segment_payload_ff_padded
```

GCC sleeps 1 ms after segment 1 and 2 ms after segment 2.

### 10.3 RGB Ex Device Resolution

GCC resolves the LED UI id from `device.db` through `GetModelLedUIID`, parsing
the database string as hexadecimal. For the observed card:

| Field | Value |
| --- | --- |
| Model | `GV-N5080AORUSM ICE-16GD` |
| Database key | `10DE_1458_418C_2C02` |
| Database `UiId` | `"20"` |
| Simple LED id | `0x20` / decimal `32` |
| Full LED id at card index 0 | `0x1020` |

The `0x20` UI class exposes a global region, a 3-by-8 fan region group, and
simple regions `5` and `6`. Its setting-region map is:

| Setting index | Region |
| --- | --- |
| `0` | Global/all |
| `1` | Fan group `2` |
| `2` | Region `5` |
| `3` | Region `6` |

### 10.4 RGB Ex Commands

RGB Ex uses 64-byte packets through `I2CApiEx`. Known commands:

| Opcode | Packet | Meaning |
| --- | --- | --- |
| `0x10` | `10 01` read 4 bytes | RGB firmware/version read; response byte `1 == 2` marks Ex-4N firmware |
| `0x11` | `11 01` read 4 bytes | Model SSID read; SSID is `(response[2] << 8) | response[3]` |
| `0x12` | effect packet | RGB LED set |
| `0x13` | `13 01` | RGB LED save |
| `0x16` | sync packet | Real-time/sync color |
| `0x17` | `17 01` read 4 bytes | App-close profile support check; `response[2] == 1` means no profile reset needed |
| `0x18` | `18 01` | App-close notification |

The RGB sync packet is:

```text
u8 opcode = 0x16
u8 subcommand = 0x01
u8 color_top_byte
u8 effect_or_mode = 0x06
u8 reserved_zero
u8 red
u8 green
u8 blue
```

### 10.5 RGB Ex LED Set Packets

All RGB Ex `0x12` LED set packets share this base layout:

```text
u8 opcode = 0x12
u8 subcommand = 0x01
u8 wire_effect_id
u8 speed
u8 brightness
u8 red
u8 green
u8 blue
u8 angle
u8 physical_packet_index
u8 color_count
u8[] rgb_triples
```

GCC maps UI effect ids to RGB Ex wire effect ids as follows:

| UI effect | Wire effect |
| --- | --- |
| `0` | `0` |
| `1` | `1` |
| `2` | `2` |
| `3` | `5` |
| `4` | `3` |
| `5` | `7` |
| `6` | `8` |
| `7` | `6` |
| `8` | `4` |
| `9` | `12` |
| `10` | `11` |
| `11` | `10` for simple ids `18`/`22`, otherwise `9` |
| `12` | `10` |
| `13` | `10` |

For simple ids `0x15`, `0x18`, `0x19`, `0x20`, `0x21`, `0x22`, and `0x23`
GCC sends six physical packets. For the current simple id `0x20`, packet indices
`0`, `1`, and `2` address the fan group, while indices `3`, `4`, and `5` address
simple selectors `2`, `3`, and `4`. The UI only supplies explicit settings for
selectors `0`, `1`, `2`, and `3`, so selector `4` falls back to the last setting.

For fan-group packets with wire effects `1`, `2`, `3`, or `4`, GCC writes eight
RGB triples starting at byte `11`. The fan source groups are reordered:

| Packet index | Fan source group |
| --- | --- |
| `0` | group `2` |
| `1` | group `0` |
| `2` | group `1` |

For wire effects `8`, `9`, `10`, and `12`, GCC writes `ClrCount` and RGB triples
from the normal color array instead.

### 10.6 Native Illumination Path

The native `GvLedLib.dll` in the same GCC install was built from a
`GvLedLib_New_241202_Add_N50_Lcd` tree and imports `GvIllumLib.dll`. That
library exports generic illumination-zone APIs and contains UTF-16 strings for:

```text
nvapi64.dll
nvldumd.dll
nvldumdx.dll
PCI\VEN_10DE&CC_0300
PCI\VEN_10DE&CC_0302
NvAPI_GPU_ClientIllumZonesGetInfo
NvAPI_GPU_ClientIllumZonesGetControl
NvAPI_GPU_ClientIllumZonesSetControl
```

This means some GCC/RGB Fusion paths use NVIDIA client illumination zones through
NVAPI. NVIDIA's public NVAPI header defines these as
`NV_GPU_CLIENT_ILLUM_ZONE_INFO_PARAMS_V1` and
`NV_GPU_CLIENT_ILLUM_ZONE_CONTROL_PARAMS_V1`, with a maximum of 32 zones and
140-byte/200-byte zone records respectively.

`GvIllumLib.dll` resolves the public NVAPI entry points through
`nvapi_QueryInterface`. The recovered QueryInterface ids are:

| QueryInterface id | Function |
| --- | --- |
| `0x4b81241b` | `NvAPI_GPU_ClientIllumZonesGetInfo` |
| `0x3dbf5764` | `NvAPI_GPU_ClientIllumZonesGetControl` |
| `0x197d065e` | `NvAPI_GPU_ClientIllumZonesSetControl` |

`GvGetIllumZonesInfo(card_index, out)` compresses the NVAPI info struct into a
0x204-byte Gigabyte struct:

```text
struct GvIllumZonesInfo {
    ZoneInfo zones[32];    // 32 * 16 bytes
    u32 num_zones;         // offset 0x200
}

struct ZoneInfo {
    u32 illum_device_idx;  // from NVAPI zone byte +0x04
    u32 provider_idx;      // from NVAPI zone byte +0x05
    u32 zone_location;     // from NVAPI zone dword +0x08
    u32 zone_type;         // from NVAPI zone dword +0x00
}
```

`GvGetIllumZonesControl(card_index, inout)` and
`GvSetIllumZonesControl(card_index, inout)` use a 0x44-byte Gigabyte zone-control
record. Offset `0x00` selects the zone index. Offsets `0x04` and `0x08` carry the
NVAPI `ctrlMode` and `type`, and the remaining fields are mode-specific control
data copied into or out of NVAPI's zone-control union. The set path first reads
the complete NVAPI control array, replaces only the selected zone, then calls
`NvAPI_GPU_ClientIllumZonesSetControl`.

`GvLedLib.dll` also has a separate native I2C path for N50 AORUS cards
(`CVgaN50AORUSLedCtrl`). This does not use `GvIllumLib.dll`; it builds
`GVDISP_I2C_REGADDR` structures and calls `GVDisplay.dll` directly. The native
helper uses:

| Field | Value |
| --- | --- |
| `nSavePort` | `0xe2` shifted address, likely Linux 7-bit `0x71` |
| `nDDCPort` | `1` |
| `nDataSize` | `8` for the recovered LED commands |

The N50 probe is a read at `0xe2` with register bytes:

```text
ab 00 00 00
```

The response is accepted when byte `0` is `0xab`; byte `1` is parsed as a BCD-ish
firmware version, and byte `3` selects the LED id:

| Response byte 3 | Full LED id |
| --- | --- |
| `0x0b` | `0x1018` |
| `0x10` | `0x1015` |
| `0x11` | `0x1019` |
| `0x13` | `0x1020` |
| `0x14` | `0x1021` |

For this card, the native N50 probe maps response tag `0x13` to the same
`0x1020` full LED id recovered from `device.db`.

The native N50 real-time color path sends an enable/effect write before the first
color write, then sends color updates as register bytes:

```text
88 01 05 63 00 01   // first enable/effect write before normal RGB updates
88 03 05 63 00 01   // alternate enable/effect write when the high color byte is non-zero
40 rr gg bb         // real-time RGB color
```

The native N50 path is therefore a third RGB transport candidate, distinct from
the managed RGB Ex `I2CApiEx` path. It is not proven on Linux yet, and should be
treated as a read-first probe target before any write support is exposed.

## 11. Windows Transport Notes

The Windows path uses `AorusLcdService.exe`, `ucVga.dll`, and `GvDisplayA.dll`.
The native layer dispatches I2C operations through private NVIDIA driver entry
points rather than Linux `/dev/i2c-*`.

Recovered private NVAPI QueryInterface ids:

```text
0x283AC65A -> NvAPI_I2CWriteEx
0x4D7B0709 -> NvAPI_I2CReadEx
```

The Windows behavior relevant to this implementation is operational: perform
setup writes separately, then refresh only metric values on the hot path.
