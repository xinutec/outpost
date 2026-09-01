{-
outpost/gate.dhall — this repository's commit gate.

Written nine commits in, which is early for a gate and was not early enough.
This is a **machine-config dependency**: `~/.config/home-manager/flake.nix`
takes it as an input and installs `packages.default`, so a bad commit here does
not fail this repository, it fails `home-manager switch` for the whole Mac —
long after the commit that caused it, and taking every other input's update down
with it, because `switch.sh` re-locks them in one step. memview's gate carries
the same argument for the same reason.

**The rows are the standard Rust four, and the fourth is the one that matters
here.** `cargo fmt`, `clippy` and `cargo test` all run against the working tree
inside the dev shell; none of them builds `packages.default`, where the git
dependency's `outputHashes` pins a hash of what `Cargo.lock` fetches. Bumping
that lock without the hash passes every other row and leaves a package that
cannot be built at all — which is exactly what took the Mac's activation down
from gamepads and thoth on 2026-08-15.

The generated `gate.json` is committed and `the table matches its Dhall`
re-renders and diffs it, the way a lockfile is checked, so running the gate
needs no `dhall` installed.
-}

let G = ../dev-lint/gate/schema.dhall

in  { name = "outpost"
    , checks =
      [ G.Check::{
        , name = "formatting"
        , argv = G.inDevShell [ "cargo", "fmt", "--all", "--check" ]
        , timeout_s = 120
        }
      , G.Check::{
        , name = "clippy"
        , argv =
            G.inDevShell
              [ "cargo", "clippy", "--all-targets", "--", "-D", "warnings" ]
        , {-  Clippy gets its own target directory: clippy-driver and rustc
              fingerprint the workspace differently and evict each other in a
              shared one, forcing a full recompile every time the gate runs —
              which reads as "the gate is slow" rather than as a bug.
          -}
          env = G.clippyTarget
        , timeout_s = 900
        }
      , {-  ⚠ **Everything worth testing here is in the library, and that is
              what the tests link.** The binary is argument parsing, printing
              and a poll loop; the rules that decide what a machine's answer
              MEANS are in `outpost::transcript` and `outpost::args`, where
              `tests/` reaches them as an outside user would.

              The fixtures are rows copied out of a real transcript rather than
              invented, because every failure mode here is silent: a parser that
              matches nothing and a session that said nothing produce the same
              empty output. That is the whole reason `endings` exists — it is
              the only exact signal that a background task finished, and it has
              to tell `killed` from `completed` on rows this repository does not
              control the format of.
          -}
        G.Check::{
        , name = "tests"
        , argv = G.inDevShell [ "cargo", "test" ]
        , timeout_s = 900
        }
      , {-  ⚠ **The packaged build, which no row above exercises.** See the
              header: this is the row that stands between a lockfile edit here
              and a Mac that cannot activate.
          -}
        G.Check::{
        , name = "the binary builds (what home-manager installs)"
        , argv = [ "nix", "build", "--no-warn-dirty", "--no-link", ".#default" ]
        , timeout_s = 1800
        }
      , G.devLint "../"
      , G.checkTable "../dev-lint"
      ]
    }
