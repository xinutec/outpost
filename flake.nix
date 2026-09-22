{
  description = "Talk to a Claude Code session running on another machine";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        outpost = pkgs.rustPlatform.buildRustPackage {
          pname = "outpost";
          version = "0.1.0";
          src = ./.;

          # The transcript rules come from another repository rather than being
          # copied in, so the lock carries a git dependency -- and a git
          # dependency needs its hash stated here, because the fetch is not
          # reproducible from the lock alone.
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "reader-0.1.0" = "sha256-MKNsBynuN0jFbJHVnf+X53vK5SHnyq856ayNItGu/ck=";
            };
          };

          # It shells out to ssh at runtime and never links it, so there is
          # nothing to add to buildInputs -- but it is worth saying out loud that
          # `ssh` and a `Host` block for the target are the real dependencies.
          meta = {
            description = "Talk to a Claude Code session running on another machine";
            mainProgram = "outpost";
          };
        };
        default = outpost;
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [ pkgs.cargo pkgs.rustc pkgs.clippy pkgs.rustfmt ];
        };
      });
    };
}
