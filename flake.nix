{
  description = "Linux control service for Gigabyte AORUS GPU LCD panels";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      home-manager,
      nixpkgs,
    }:
    let
      inherit (nixpkgs) lib;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = lib.genAttrs supportedSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
      localSystem = "x86_64-linux";
      localPkgs = pkgsFor localSystem;
      localHomeModule = {
        home = {
          username = "example";
          homeDirectory = "/home/example";
          stateVersion = "26.11";
        };

        services.gigabyte-lcd = {
          enable = true;
          mascot = "/home/example/.config/gigabyte-lcd/background.png";
          bus = 1;
          addr = "0x61";
          deviceId = "0x21";
          imageSettleDelay = 20;
          overlayInterval = 4;
          logLevel = "info";
          systemdTargets = [ ];
        };
      };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          gigabyte-lcd = pkgs.rustPlatform.buildRustPackage {
            pname = "gigabyte-lcd";
            version = "0.1.0";

            src = self;

            cargoLock.lockFile = ./Cargo.lock;

            meta = {
              description = "Linux control service for Gigabyte AORUS GPU LCD panels";
              homepage = "https://github.com/PrivateGER/Gigabyte-Aorus-LCD-Driver";
              license = lib.licenses.mit;
              mainProgram = "gigabyte-lcd";
              platforms = lib.platforms.linux;
            };
          };
        in
        {
          default = gigabyte-lcd;
          inherit gigabyte-lcd;
        }
      );

      apps = forAllSystems (
        system:
        let
          program = "${self.packages.${system}.default}/bin/gigabyte-lcd";
          app = {
            type = "app";
            inherit program;
            meta.description = "Run the Gigabyte AORUS GPU LCD control service";
          };
        in
        {
          default = app;
          gigabyte-lcd = app;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = self.packages.${system}.default;
          home-manager-module =
            (home-manager.lib.homeManagerConfiguration {
              inherit pkgs;
              modules = [
                self.homeModules.default
                {
                  home = {
                    username = "gigabyte-lcd-test";
                    homeDirectory = "/tmp/gigabyte-lcd-test";
                    stateVersion = "26.11";
                  };

                  services.gigabyte-lcd = {
                    enable = true;
                    mascot = "/tmp/background.png";
                    systemdTargets = [ ];
                  };
                }
              ];
            }).activationPackage;
        }
        // lib.optionalAttrs (system == localSystem) {
          local-home-manager = self.homeConfigurations.example-local.activationPackage;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              rustc
              rustfmt
            ];

            RUST_BACKTRACE = "1";
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);

      homeModules.default = import ./nix/home-manager.nix self;

      homeConfigurations.example-local = home-manager.lib.homeManagerConfiguration {
        pkgs = localPkgs;
        modules = [
          self.homeModules.default
          localHomeModule
        ];
      };
    };
}
