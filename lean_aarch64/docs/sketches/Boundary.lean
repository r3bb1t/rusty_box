/-! A design sketch, checked by `lean docs/sketches/Boundary.lean` (Lean 4.35); not part of the library.

Two kinds of guarantee about a guest's run, and why they differ.

1. Enforcement at the boundary (a statement about the box, for every guest):
   every way out of the machine is one closed type, leaving through any of
   them needs the proof that the policy allows it, and the only source of that
   proof is the decision at the boundary. A run's output is then *typed* as
   allowed output, whatever the guest does, and the machine halts at the step
   where the guest would first break the policy, before anything leaves.
   (Schneider, "Enforceable Security Policies": safety properties, enforced by
   halting just before a violation; complete mediation by construction.)

2. Verification of one replay (a statement about one guest's run): with the
   inputs fixed the run is one trace, and a property of it is decidable. The
   kernel can check it by evaluation only for short runs. -/

/-! ## The machine and its one boundary -/

/-- What can leave the machine: one closed type. A device the board gains is a
new constructor here, so no output can be added that bypasses the policy. -/
inductive Egress where
  | serial (byte : UInt8)
  | frame (dest : UInt32) (payload : List UInt8)
  deriving DecidableEq, Repr

/-- A guest is any program at all: a state and a step that may ask for output.
Nothing about Linux, or any particular guest, is assumed. -/
structure Guest where
  State : Type
  start : State
  step : State → State × Option Egress

/-- A policy is a decidable predicate on what leaves. -/
structure Policy where
  Allowed : Egress → Prop
  dec : DecidablePred Allowed

/-- Output that has left the machine: an egress *with* the proof that the
policy allowed it. There is no other way to make one. -/
abbrev Sent (p : Policy) := { e : Egress // p.Allowed e }

/-- Where a run ended: it ran out of steps, or it halted at the step whose
output the policy refused, which is kept (for whoever examines the guest) but
never sent. -/
inductive Ending (p : Policy) where
  | ranOut
  | refused (e : Egress) (h : ¬ p.Allowed e)

structure Run (g : Guest) (p : Policy) where
  state : g.State
  sent : List (Sent p)
  ending : Ending p

/-- The boundary: the only place an `Egress` becomes `Sent`, by the decision. -/
def run (g : Guest) (p : Policy) : Nat → g.State → List (Sent p) → Run g p
  | 0, s, out => ⟨s, out.reverse, .ranOut⟩
  | n + 1, s, out =>
    have := p.dec
    match g.step s with
    | (s', none) => run g p n s' out
    | (s', some e) =>
      if h : p.Allowed e then run g p n s' (⟨e, h⟩ :: out)
      else ⟨s, out.reverse, .refused e h⟩     -- halts *before* `e` leaves

/-- For every guest, every policy, every run length: everything that left was
allowed. The type of `sent` already says so; this only reads it back. -/
theorem everything_sent_is_allowed (g : Guest) (p : Policy) (n : Nat) :
    ∀ e ∈ (run g p n g.start []).sent, p.Allowed e.val :=
  fun e _ => e.property

/-! ## A guest, a policy, a run -/

/-- A toy guest: counts up and, at step 3, asks to send a frame to 10.0.0.66,
otherwise serial bytes. -/
def toy : Guest where
  State := Nat
  start := 0
  step n := (n + 1, if n = 3 then some (.frame 0x0A000042 [1, 2]) else some (.serial n.toUInt8))

def noFramesTo (dest : UInt32) : Policy where
  Allowed e := ∀ p, e ≠ .frame dest p
  dec e := match e with
    | .serial _ => isTrue (fun _ h => nomatch h)
    | .frame d q =>
      if hd : d = dest then isFalse (fun h => h q (by rw [hd]))
      else isTrue (fun _ h => by cases h; exact hd rfl)

def r := run toy (noFramesTo 0x0A000042) 10 toy.start []
#eval r.sent.map (·.val)                       -- the three serial bytes before the frame
#eval match r.ending with | .refused e _ => s!"halted before {repr e}" | .ranOut => "ran out"

/-! ## Verifying one replay, by the kernel -/

/-- The trace of the first `n` outputs a guest asks for. -/
def outputs (g : Guest) : Nat → g.State → List (Option Egress)
  | 0, _ => []
  | n + 1, s => let (s', o) := g.step s; o :: outputs g n s'

/-- A different toy guest that only ever writes serial bytes. -/
def quiet : Guest where
  State := Nat
  start := 0
  step n := (n + 1, some (.serial n.toUInt8))

/-- A property of one run, proved by the kernel evaluating the run: in its first
200 steps this guest asks for no frame at all. This works because the run is
short; for billions of steps the kernel cannot evaluate it, and the property is
then checked by a proved monitor in compiled code instead. -/
def isFrame : Option Egress → Bool | some (.frame ..) => true | _ => false

theorem quiet_never_frames : (outputs quiet 200 quiet.start).all (fun o => !isFrame o) = true := by
  decide

/-- What the check means, for anyone who reads the theorem: no output in the
run is a frame. -/
theorem quiet_never_frames' : ∀ o ∈ outputs quiet 200 quiet.start, ∀ d q, o ≠ some (.frame d q) := by
  intro o ho d q he
  have := List.all_eq_true.mp quiet_never_frames o ho
  subst he; simp [isFrame] at this

#print axioms everything_sent_is_allowed
#print axioms quiet_never_frames'
