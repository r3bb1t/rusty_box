import Lake
open Lake DSL

package leanbox where
  moreLeancArgs := #["-O3", "-DNDEBUG"]

lean_lib Leanbox where

@[default_target]
lean_exe leanbox where
  root := `Main
  moreLeancArgs := #["-O3", "-DNDEBUG"]
