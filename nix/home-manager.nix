self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.gigabyte-lcd;

  pathOrString = with lib.types; either path str;

  commandArgs = [
    "${cfg.package}/bin/gigabyte-lcd"
    "--mascot"
    (toString cfg.mascot)
    "--bus"
    (toString cfg.bus)
    "--addr"
    cfg.addr
    "--device-id"
    cfg.deviceId
    "--transport"
    cfg.transport
    "--i2c-speed-khz"
    (toString cfg.i2cSpeedKHz)
    "--image-settle-delay"
    (toString cfg.imageSettleDelay)
    "--metrics"
    (lib.concatStringsSep "," cfg.metrics)
    "--overlay-interval"
    (toString cfg.overlayInterval)
    "--update-interval"
    (toString cfg.updateInterval)
    "--gpu-index"
    (toString cfg.gpuIndex)
    "--log-level"
    cfg.logLevel
  ]
  ++ lib.optionals (cfg.gif != null) [
    "--gif"
    (toString cfg.gif)
  ]
  ++ lib.optional cfg.noMonitoring "--no-monitoring"
  ++ cfg.extraArgs;
in
{
  options.services.gigabyte-lcd = {
    enable = lib.mkEnableOption "the Gigabyte AORUS GPU LCD user service";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "inputs.gigabyte-lcd.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "Package providing the gigabyte-lcd binary.";
    };

    mascot = lib.mkOption {
      type = lib.types.nullOr pathOrString;
      default = null;
      example = "~/.config/gigabyte-lcd/background.png";
      description = "PNG image to place on the left side in default run mode.";
    };

    gif = lib.mkOption {
      type = lib.types.nullOr pathOrString;
      default = null;
      example = "~/.config/gigabyte-lcd/animation.gif";
      description = "Optional GIF to normalize and play panel-side.";
    };

    bus = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 1;
      description = "Linux I2C bus.";
    };

    addr = lib.mkOption {
      type = lib.types.str;
      default = "0x61";
      description = "7-bit I2C address.";
    };

    deviceId = lib.mkOption {
      type = lib.types.str;
      default = "0x21";
      description = "LCD device LED id.";
    };

    transport = lib.mkOption {
      type = lib.types.enum [
        "auto"
        "rm"
        "i2c-dev"
      ];
      default = "auto";
      description = "I2C transport backend.";
    };

    i2cSpeedKHz = lib.mkOption {
      type = lib.types.enum [
        100
        200
        300
        400
      ];
      default = 400;
      description = "RM transport bus speed in kHz.";
    };

    imageSettleDelay = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 5;
      description = "Extra delay after image upload, in seconds.";
    };

    metrics = lib.mkOption {
      type = lib.types.listOf (
        lib.types.enum [
          "temp"
          "clock"
          "usage"
          "fan"
          "vram-clock"
          "vram"
          "power"
          "all"
          "none"
        ]
      );
      default = [
        "temp"
        "usage"
        "power"
      ];
      description = "Overlay metrics to display.";
    };

    overlayInterval = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 4;
      description = "Vendor overlay rotation interval.";
    };

    updateInterval = lib.mkOption {
      type = lib.types.ints.positive;
      default = 1;
      description = "Seconds between metric refresh checks.";
    };

    noMonitoring = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Upload the image or GIF and exit without NVML metric monitoring.";
    };

    gpuIndex = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 0;
      description = "NVML GPU index.";
    };

    logLevel = lib.mkOption {
      type = lib.types.enum [
        "info"
        "debug"
      ];
      default = "info";
      description = "Service log level.";
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Additional arguments appended to the gigabyte-lcd command.";
    };

    systemdTargets = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "default.target" ];
      example = [ ];
      description = "User systemd targets that should start the service. Set to an empty list to install the unit without enabling it.";
    };

    restart = lib.mkOption {
      type = lib.types.str;
      default = "on-failure";
      description = "systemd Restart policy.";
    };

    restartSec = lib.mkOption {
      type = lib.types.ints.unsigned;
      default = 5;
      description = "Seconds to wait before restarting the service.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.mascot != null;
        message = "services.gigabyte-lcd.mascot must be set.";
      }
    ];

    systemd.user.services.gigabyte-lcd-rust = {
      Unit = {
        Description = "Gigabyte GPU LCD Rust service";
        After = [ "graphical-session.target" ];
      };

      Service = {
        Type = "simple";
        ExecStart = lib.escapeShellArgs commandArgs;
        Restart = cfg.restart;
        RestartSec = cfg.restartSec;
      };

      Install = lib.optionalAttrs (cfg.systemdTargets != [ ]) {
        WantedBy = cfg.systemdTargets;
      };
    };
  };
}
