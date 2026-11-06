import LeanboxTest.Dev.Flash
import LeanboxTest.Dev.Gic
import LeanboxTest.Dev.Pl011
import LeanboxTest.Dev.Pl031
import LeanboxTest.Dev.Pl061
import LeanboxTest.Dev.VirtioMmio
/-!
# Leanbox's tests

Each device's unit tests, ported from rusty_box_aarch64's, as `#guard`s:
building this library (`lake test`) evaluates every one, and a failing
check fails the build. The processor has no tests here: it is checked
against rusty_box in lockstep (`Leanbox.Lockstep`), whole boots and random
single instructions alike.
-/
