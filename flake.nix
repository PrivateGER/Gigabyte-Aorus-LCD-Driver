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
    };
}
