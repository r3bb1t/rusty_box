/-! A design sketch, checked by `lean docs/sketches/Syscalls.lean` (Lean 4.35); not part of the library.

Linux system-call hooks on arm64: the call table as a type, each call's
arguments a type computed from the call and the machine state, decoded by
certified readers of guest memory. Numbers and flags: the asm-generic table
arm64 uses (include/uapi/asm-generic/unistd.h, asm-generic/fcntl.h). -/

inductive El | el0 | el1 deriving DecidableEq

/-- The architectural view: `read` is a byte through the current translation
with the current exception level's permission, `none` where it would fault. -/
structure State where
  el : El
  x : Fin 31 → UInt64
  read : UInt64 → Option UInt8

/-! ## Certified views of guest memory -/

/-- `bs` are the bytes at `a`, each readable. -/
inductive BytesAt (s : State) : UInt64 → List UInt8 → Prop
  | nil  {a} : BytesAt s a []
  | cons {a b bs} : s.read a = some b → BytesAt s (a + 1) bs → BytesAt s a (b :: bs)

abbrev Bytes (s : State) (a : UInt64) (n : Nat) := { bs : List UInt8 // BytesAt s a bs ∧ bs.length = n }

def readBytes (s : State) : (n : Nat) → (a : UInt64) → Option (Bytes s a n)
  | 0, _ => some ⟨[], .nil, rfl⟩
  | n + 1, a =>
    match h : s.read a with
    | none => none
    | some b => match readBytes s n (a + 1) with
      | none => none
      | some ⟨bs, hb, hl⟩ => some ⟨b :: bs, .cons h hb, by simp [hl]⟩

/-- `bs` is the NUL-terminated string at `a`. -/
inductive CStrAt (s : State) : UInt64 → List UInt8 → Prop
  | nil  {a} : s.read a = some 0 → CStrAt s a []
  | cons {a b bs} : s.read a = some b → b ≠ 0 → CStrAt s (a + 1) bs → CStrAt s a (b :: bs)

abbrev CStr (s : State) (a : UInt64) (max : Nat) := { bs : List UInt8 // CStrAt s a bs ∧ bs.length < max }

def scan (s : State) : (max : Nat) → (a : UInt64) → Option (CStr s a max)
  | 0, _ => none
  | max + 1, a =>
    match h : s.read a with
    | none => none
    | some b =>
      if hb : b = 0 then some ⟨[], .nil (hb ▸ h), by simp⟩
      else match scan s max (a + 1) with
        | none => none
        | some ⟨bs, hc, hl⟩ => some ⟨b :: bs, .cons h hb hc, by simp; omega⟩

/-- A little-endian word from its bytes. -/
def le64 (bs : List UInt8) : UInt64 := bs.foldr (fun b acc => (acc <<< 8) ||| b.toUInt64) 0

/-- `v` is the 64-bit little-endian word at `a` (a guest pointer). -/
def U64At (s : State) (a v : UInt64) : Prop := ∃ bs, BytesAt s a bs ∧ bs.length = 8 ∧ le64 bs = v

def readU64 (s : State) (a : UInt64) : Option { v : UInt64 // U64At s a v } :=
  (readBytes s 8 a).map fun ⟨bs, hb, hl⟩ => ⟨le64 bs, bs, hb, hl, rfl⟩

/-- `strs` is the NULL-terminated array of string pointers at `a` (argv, envp). -/
inductive StrArrayAt (s : State) : UInt64 → List (List UInt8) → Prop
  | nil  {a} : U64At s a 0 → StrArrayAt s a []
  | cons {a p str strs} : U64At s a p → p ≠ 0 → CStrAt s p str →
      StrArrayAt s (a + 8) strs → StrArrayAt s a (str :: strs)

abbrev StrArray (s : State) (a : UInt64) := { strs : List (List UInt8) // StrArrayAt s a strs }

def readStrArray (s : State) (strMax : Nat) : (count : Nat) → (a : UInt64) → Option (StrArray s a)
  | 0, _ => none
  | count + 1, a => match readU64 s a with
    | none => none
    | some ⟨p, hp⟩ =>
      if h0 : p = 0 then some ⟨[], .nil (h0 ▸ hp)⟩
      else match scan s strMax p, readStrArray s strMax count (a + 8) with
        | some ⟨str, hc, _⟩, some ⟨strs, ht⟩ => some ⟨str :: strs, .cons hp h0 hc ht⟩
        | _, _ => none

/-! ## The ABI's own types -/

/-- A directory argument: `AT_FDCWD` (-100) or a descriptor. -/
inductive Dirfd | cwd | fd (n : UInt32)
def Dirfd.ofReg (x : UInt64) : Dirfd := if x.toUInt32 = 0xFFFFFF9C then .cwd else .fd x.toUInt32

inductive Access | rdonly | wronly | rdwr
/-- `open` flags, decoded; an access mode of 3 is not one (`O_ACCMODE`). -/
structure OpenFlags where
  access : Access
  creat : Bool
  trunc : Bool
  append : Bool
def OpenFlags.ofReg (x : UInt64) : Option OpenFlags :=
  let acc := match x &&& 3 with | 0 => some Access.rdonly | 1 => some .wronly | 2 => some .rdwr | _ => none
  acc.map fun access => { access, creat := x &&& 0o100 != 0, trunc := x &&& 0o1000 != 0,
                          append := x &&& 0o2000 != 0 }

def PATH_MAX : Nat := 4096
def ARG_MAX : Nat := 256          -- strings per argv, for the sketch

/-! ## The call table as a type -/

inductive Syscall | openat | close | read | write | exitGroup | execve
  deriving DecidableEq

def Syscall.number : Syscall → UInt64
  | .openat => 56 | .close => 57 | .read => 63 | .write => 64 | .exitGroup => 94 | .execve => 221

/-- The call a number names, with the proof that it names it. -/
def Syscall.ofNumber (n : UInt64) : Option { sc : Syscall // sc.number = n } :=
  [Syscall.openat, .close, .read, .write, .exitGroup, .execve].findSome? fun sc =>
    if h : sc.number = n then some ⟨sc, h⟩ else none

/-- Each call's arguments, as types that depend on the state they were read in. -/
structure OpenatArgs (s : State) where
  dirfd : Dirfd
  path : CStr s (s.x 1) PATH_MAX            -- the string x1 points to, with its proof
  flags : OpenFlags
  mode : UInt32

structure WriteArgs (s : State) (cap : Nat) where
  fd : UInt32
  /-- exactly as many bytes as x2 says, up to the hook's cap, at x1 -/
  buf : Bytes s (s.x 1) (min (s.x 2).toNat cap)

structure ReadArgs (s : State) where
  fd : UInt32
  buf : UInt64                              -- where the kernel will write: not read here
  count : UInt64

structure ExecveArgs (s : State) where
  path : CStr s (s.x 0) PATH_MAX
  argv : StrArray s (s.x 1)                 -- every string, through every pointer

def Syscall.Args (s : State) (cap : Nat) : Syscall → Type
  | .openat => OpenatArgs s
  | .close => UInt32
  | .read => ReadArgs s
  | .write => WriteArgs s cap
  | .exitGroup => Int32
  | .execve => ExecveArgs s

/-- Decode the arguments of `sc` from x0–x5 and guest memory. -/
def Syscall.decode (s : State) (cap : Nat) : (sc : Syscall) → Option (sc.Args s cap)
  | .openat => do
      let path ← scan s PATH_MAX (s.x 1)
      let flags ← OpenFlags.ofReg (s.x 2)
      pure { dirfd := .ofReg (s.x 0), path, flags, mode := (s.x 3).toUInt32 }
  | .close => some (s.x 0).toUInt32
  | .read => some { fd := (s.x 0).toUInt32, buf := s.x 1, count := s.x 2 }
  | .write => (readBytes s _ (s.x 1)).map fun buf => { fd := (s.x 0).toUInt32, buf }
  | .exitGroup => some (s.x 0).toUInt32.toInt32
  | .execve => do
      let path ← scan s PATH_MAX (s.x 0)
      let argv ← readStrArray s PATH_MAX ARG_MAX (s.x 1)
      pure { path, argv }

/-- What a hook on the call `sc` is given: the call was made from user mode,
x8 named it, and its arguments as that call's types. The `svc` itself is the
static half (L18), decided when the trace is built. -/
structure CallAt (s : State) (cap : Nat) (sc : Syscall) where
  user : s.el = .el0
  named : s.x 8 = sc.number
  args : sc.Args s cap

/-- Any call: which one, and its witness. Matching on the pair tells each case
which call it is, and so the type of its arguments. -/
abbrev SyscallAt (s : State) (cap : Nat) := (sc : Syscall) × CallAt s cap sc

def syscallAt (s : State) (cap : Nat) : Option (SyscallAt s cap) :=
  if hu : s.el = .el0 then do
    let ⟨sc, hn⟩ ← Syscall.ofNumber (s.x 8)
    let args ← sc.decode s cap
    pure ⟨sc, { user := hu, named := hn.symm, args }⟩
  else none

/-! ## Hooks -/

def str (bs : List UInt8) : String := String.fromUTF8! ⟨bs.toArray⟩
def quote (bs : List UInt8) : String := (repr (str bs)).pretty
def hex (x : UInt64) : String := "0x" ++ String.ofList (Nat.toDigits 16 x.toNat)

/-- A typed strace: one handler, and in each case the arguments are that call's.
The match is exhaustive: a call added to `Syscall` breaks it until it is printed. -/
def strace (s : State) : SyscallAt s 64 → String
  | ⟨.openat, w⟩ =>
    let a := w.args
    let dir := match a.dirfd with | .cwd => "AT_FDCWD" | .fd n => toString n
    let acc := match a.flags.access with | .rdonly => "O_RDONLY" | .wronly => "O_WRONLY" | .rdwr => "O_RDWR"
    let more := (if a.flags.creat then "|O_CREAT" else "") ++ (if a.flags.trunc then "|O_TRUNC" else "")
    s!"openat({dir}, {quote a.path.val}, {acc}{more})"
  | ⟨.close, w⟩ => s!"close({show UInt32 from w.args})"
  | ⟨.read, w⟩ => s!"read({w.args.fd}, {hex w.args.buf}, {w.args.count})"
  | ⟨.write, w⟩ => s!"write({w.args.fd}, {quote w.args.buf.val}, {s.x 2})"
  | ⟨.exitGroup, w⟩ => s!"exit_group({show Int32 from w.args})"
  | ⟨.execve, w⟩ => s!"execve({quote w.args.path.val}, {w.args.argv.val.map str})"

/-- A hook on one call takes that call's witness: here, every path a user
program opens. Its type admits no other call, so it has nothing to take apart. -/
def opens (s : State) (w : CallAt s 64 .openat) : String := str w.args.path.val

/-- The witness's proofs are usable: the path is a string the user program
could itself read, and x8 is 56. -/
example (s : State) (w : CallAt s 64 .openat) : s.x 8 = 56 := w.named
example (s : State) (w : CallAt s 64 .openat) : CStrAt s (s.x 1) w.args.path.val := w.args.path.property.1

/-! ## A user program's calls -/

/-- Guest memory: strings at addresses, and argv's pointer array at 0x5000. -/
def mem (a : UInt64) : Option UInt8 :=
  let strs : List (UInt64 × String) :=
    [(0x2000, "/etc/passwd"), (0x3000, "hello\n"), (0x4000, "/bin/ls"), (0x4100, "ls"), (0x4200, "-l")]
  let ptrs : List UInt64 := [0x4100, 0x4200, 0]
  match strs.find? fun (base, t) => base ≤ a && a ≤ base + t.utf8ByteSize.toUInt64 with
  | some (base, t) => some (if a = base + t.utf8ByteSize.toUInt64 then 0 else t.toUTF8[(a - base).toNat]!)
  | none =>
    if 0x5000 ≤ a && a < 0x5000 + 8 * 3 then
      let p := ptrs[((a - 0x5000) / 8).toNat]!
      some ((p >>> (8 * ((a - 0x5000) % 8))).toUInt8)
    else none

def call (el : El) (n : UInt64) (args : List UInt64) : State where
  el := el
  x := fun r => if r.val = 8 then n else args.getD r.val 0
  read := mem

#eval (syscallAt (call .el0 56 [0xFFFFFFFFFFFFFF9C, 0x2000, 0]) 64).map (strace _)
#eval (syscallAt (call .el0 64 [1, 0x3000, 6]) 64).map (strace _)
#eval (syscallAt (call .el0 221 [0x4000, 0x5000]) 64).map (strace _)
#eval (syscallAt (call .el0 94 [0]) 64).map (strace _)
-- the one-call hook, given only openat witnesses
#eval match syscallAt (call .el0 56 [0xFFFFFFFFFFFFFF9C, 0x2000, 0]) 64 with
  | some ⟨.openat, w⟩ => some (opens _ w) | _ => none
-- the same openat from the kernel (EL1) is not a system call
#eval (syscallAt (call .el1 56 [0xFFFFFFFFFFFFFF9C, 0x2000, 0]) 64).map (strace _)
-- an access mode of 3 is not an open flag set: no witness
#eval (syscallAt (call .el0 56 [0xFFFFFFFFFFFFFF9C, 0x2000, 3]) 64).map (strace _)

#print axioms syscallAt

-- a hook on openat cannot be handed a write
/-- error: Application type mismatch: The argument
  w
has type
  CallAt s 64 Syscall.write
but is expected to have type
  CallAt s 64 Syscall.openat
in the application
  opens s w -/
#guard_msgs in
example (s : State) (w : CallAt s 64 .write) : String := opens s w
-- accepted twin: the same call with an openat witness
example (s : State) (w : CallAt s 64 .openat) : String := opens s w
