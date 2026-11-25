{
  description = "meshcore-cli-rs - command line interface to MeshCore companion radios and repeaters";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    # builds the git dependency on meshcore-rs without manual output hashes
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      # nixpkgs 26.11 dropped x86_64-darwin
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      build = pkgs:
        let
          inherit (pkgs) lib;
          craneLib = crane.mkLib pkgs;
          commonArgs = {
            src = craneLib.cleanCargoSource ./.;
            strictDeps = true;
            nativeBuildInputs = [ pkgs.pkg-config ];
            # D-Bus for BLE through BlueZ
            buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.dbus ];
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          inherit craneLib commonArgs cargoArtifacts;
          package = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            meta.mainProgram = "meshcore-cli-rs";
          });
        };
    in
    {
      packages = forAllSystems (pkgs: { default = (build pkgs).package; });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe (build pkgs).package;
          meta.description = "Command line interface to MeshCore companion radios and repeaters";
        };
      });

      checks = forAllSystems (pkgs:
        let b = build pkgs; in
        {
          package = b.package;
          clippy = b.craneLib.cargoClippy (b.commonArgs // {
            inherit (b) cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- -D warnings";
          });
          fmt = b.craneLib.cargoFmt { inherit (b.commonArgs) src; };
          test = b.craneLib.cargoTest (b.commonArgs // { inherit (b) cargoArtifacts; });
        });

      devShells = forAllSystems (pkgs: {
        default = (build pkgs).craneLib.devShell {
          checks = self.checks.${pkgs.stdenv.hostPlatform.system};
          packages = [ pkgs.rust-analyzer ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
