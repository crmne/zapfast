{
  description = "Fast native WhatsApp client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # rust-toolchain.toml pins the compiler so local builds and CI agree.
    # This reads that file rather than restating the version here.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            rust-analyzer
            pkg-config
            cmake
            perl
            alsa-lib
            libxkbcommon
            wayland
            libGL
            libx11
            libxcursor
            libxi
            libxrandr
          ];
          # The GUI dlopens its Wayland, X11 and GL libraries at run time.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
            with pkgs;
            [
              libxkbcommon
              wayland
              libGL
              libx11
              libxcursor
              libxi
              libxrandr
            ]
          );
          ZAPFAST_TEST_RTL_FONT = "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSans.ttf";
        };
      });

      packages = forAllSystems (
        pkgs:
        let
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
          runtimeLibs = with pkgs; [
            libxkbcommon
            wayland
            libGL
            libx11
            libxcursor
            libxi
            libxrandr
          ];
          zapfast = rustPlatform.buildRustPackage rec {
            pname = "zapfast";
            version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
            src = self;

            # Import registry dependencies directly from Cargo.lock so ordinary
            # lock-file updates do not require refreshing a vendor hash. Git
            # dependencies remain pinned to their exact locked revisions.
            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };

            nativeBuildInputs = with pkgs; [
              pkg-config
              cmake
              perl
              makeWrapper
            ];
            buildInputs = with pkgs; [
              alsa-lib
              libGL
              libx11
            ];
            ZAPFAST_TEST_RTL_FONT = "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSans.ttf";

            # The GUI dlopens its Wayland, X11 and GL libraries at run time.
            postFixup = ''
              wrapProgram $out/bin/zapfast \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
            '';

            postInstall = ''
              install -Dm644 packaging/applications/zapfast.desktop \
                $out/share/applications/zapfast.desktop
              install -Dm644 packaging/icons/zapfast.svg \
                $out/share/icons/hicolor/scalable/apps/zapfast.svg
              install -Dm644 contrib/omarchy/zapfast.json.tpl \
                $out/share/zapfast/omarchy/zapfast.json.tpl
              install -Dm755 contrib/omarchy/zapfast-theme \
                $out/share/zapfast/omarchy/zapfast-theme
            '';

            meta = {
              description = "Fast native WhatsApp client";
              homepage = "https://zapfast.rocks";
              license = pkgs.lib.licenses.mit;
              mainProgram = "zapfast";
              platforms = pkgs.lib.platforms.linux;
            };
          };
        in
        {
          default = zapfast;
          inherit zapfast;
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
