{-
outpost/acting.dhall — every operational script in this repository (`*.sh`,
`*.py`), classified, with a ceiling per class; a test needs no row when its path
says it is one. What the classes mean and when a ceiling may rise:
dev-lint/tables/acting_schema.dhall. The gate runs `dev-lint-fleet --acting .`:
a script with no row, a row for a file that is gone, or a class over its
ceiling fails the commit that causes it.
-}
let A = ../dev-lint/tables/acting_schema.dhall

let subject = A.subject

let Class = A.Class

in  { ceilings =
      { acts = 0
      , bootstrap = 0
      , by_design = 0
      , observes = 0
      , observer_checks = 0
      }
    , subjects =
      [ subject "scripts/setup-hooks.sh" Class.OutOfScope ""
      ]
    }
