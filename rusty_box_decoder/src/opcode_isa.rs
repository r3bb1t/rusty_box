//! Per-opcode CPUID/ISA feature gate — GENERATED, DO NOT EDIT BY HAND.
//!
//! Regenerate with `python scripts/gen_opcode_isa.py` after syncing
//! `cpp_orig/bochs/` or adding `Opcode` / `X86Feature` variants.
//!
//! Mirrors the ISA field of Bochs `cpu/decoder/ia_opcodes.def`, which
//! `init_FetchDecodeTables()` uses to point unsupported opcodes at
//! `BxError`. Indexed by `Opcode as usize`; `ISA_ALWAYS` marks an
//! instruction with no feature gate (base ISA), which is also the
//! conservative fallback for the few opcodes Bochs does not define.

use crate::features::X86Feature;
use crate::opcode::Opcode;

/// Sentinel: this opcode is not gated on any CPUID feature.
pub const ISA_ALWAYS: u16 = 0xFFFF;

/// `X86Feature as u16` required by each opcode (2908 of 3679 are gated).
pub static OPCODE_ISA: [u16; 3679] = [
    ISA_ALWAYS, // IaError
    ISA_ALWAYS, // InsertedOpcode
    ISA_ALWAYS, // Aaa
    ISA_ALWAYS, // Aad
    ISA_ALWAYS, // Aam
    ISA_ALWAYS, // Aas
    ISA_ALWAYS, // Daa
    ISA_ALWAYS, // Das
    ISA_ALWAYS, // AdcEbGb
    ISA_ALWAYS, // AndEbGb
    ISA_ALWAYS, // AddEbGb
    ISA_ALWAYS, // CmpEbGb
    ISA_ALWAYS, // OrEbGb
    ISA_ALWAYS, // SbbEbGb
    ISA_ALWAYS, // SubEbGb
    ISA_ALWAYS, // TestEbGb
    ISA_ALWAYS, // XorEbGb
    ISA_ALWAYS, // AdcEwGw
    ISA_ALWAYS, // AddEwGw
    ISA_ALWAYS, // AndEwGw
    ISA_ALWAYS, // CmpEwGw
    ISA_ALWAYS, // OrEwGw
    ISA_ALWAYS, // SbbEwGw
    ISA_ALWAYS, // SubEwGw
    ISA_ALWAYS, // TestEwGw
    ISA_ALWAYS, // XorEwGw
    ISA_ALWAYS, // AdcEdGd
    ISA_ALWAYS, // AddEdGd
    ISA_ALWAYS, // AndEdGd
    ISA_ALWAYS, // CmpEdGd
    ISA_ALWAYS, // OrEdGd
    ISA_ALWAYS, // SbbEdGd
    ISA_ALWAYS, // SubEdGd
    ISA_ALWAYS, // TestEdGd
    ISA_ALWAYS, // XorEdGd
    ISA_ALWAYS, // AdcAlib
    ISA_ALWAYS, // AddAlib
    ISA_ALWAYS, // AndAlib
    ISA_ALWAYS, // CmpAlib
    ISA_ALWAYS, // OrAlib
    ISA_ALWAYS, // SbbAlib
    ISA_ALWAYS, // SubAlib
    ISA_ALWAYS, // TestAlib
    ISA_ALWAYS, // XorAlib
    ISA_ALWAYS, // AdcAxiw
    ISA_ALWAYS, // AddAxiw
    ISA_ALWAYS, // AndAxiw
    ISA_ALWAYS, // CmpAxiw
    ISA_ALWAYS, // OrAxiw
    ISA_ALWAYS, // SbbAxiw
    ISA_ALWAYS, // SubAxiw
    ISA_ALWAYS, // TestAxiw
    ISA_ALWAYS, // XorAxiw
    ISA_ALWAYS, // AdcEaxid
    ISA_ALWAYS, // AddEaxid
    ISA_ALWAYS, // AndEaxid
    ISA_ALWAYS, // CmpEaxid
    ISA_ALWAYS, // OrEaxid
    ISA_ALWAYS, // SbbEaxid
    ISA_ALWAYS, // SubEaxid
    ISA_ALWAYS, // TestEaxid
    ISA_ALWAYS, // XorEaxid
    ISA_ALWAYS, // AddEbIb
    ISA_ALWAYS, // OrEbIb
    ISA_ALWAYS, // AdcEbIb
    ISA_ALWAYS, // SbbEbIb
    ISA_ALWAYS, // AndEbIb
    ISA_ALWAYS, // SubEbIb
    ISA_ALWAYS, // XorEbIb
    ISA_ALWAYS, // TestEbIb
    ISA_ALWAYS, // CmpEbIb
    ISA_ALWAYS, // AddEwIw
    ISA_ALWAYS, // OrEwIw
    ISA_ALWAYS, // AdcEwIw
    ISA_ALWAYS, // SbbEwIw
    ISA_ALWAYS, // AndEwIw
    ISA_ALWAYS, // SubEwIw
    ISA_ALWAYS, // XorEwIw
    ISA_ALWAYS, // TestEwIw
    ISA_ALWAYS, // CmpEwIw
    ISA_ALWAYS, // AddEwsIb
    ISA_ALWAYS, // OrEwsIb
    ISA_ALWAYS, // AdcEwsIb
    ISA_ALWAYS, // SbbEwsIb
    ISA_ALWAYS, // AndEwsIb
    ISA_ALWAYS, // SubEwsIb
    ISA_ALWAYS, // XorEwsIb
    ISA_ALWAYS, // TestEwsIb
    ISA_ALWAYS, // CmpEwsIb
    ISA_ALWAYS, // AddEdId
    ISA_ALWAYS, // OrEdId
    ISA_ALWAYS, // AdcEdId
    ISA_ALWAYS, // SbbEdId
    ISA_ALWAYS, // AndEdId
    ISA_ALWAYS, // SubEdId
    ISA_ALWAYS, // XorEdId
    ISA_ALWAYS, // TestEdId
    ISA_ALWAYS, // CmpEdId
    ISA_ALWAYS, // AddEdsIb
    ISA_ALWAYS, // OrEdsIb
    ISA_ALWAYS, // AdcEdsIb
    ISA_ALWAYS, // SbbEdsIb
    ISA_ALWAYS, // AndEdsIb
    ISA_ALWAYS, // SubEdsIb
    ISA_ALWAYS, // XorEdsIb
    ISA_ALWAYS, // TestEdsIb
    ISA_ALWAYS, // CmpEdsIb
    ISA_ALWAYS, // XorEwGwZeroIdiom
    ISA_ALWAYS, // XorGwEwZeroIdiom
    ISA_ALWAYS, // XorEdGdZeroIdiom
    ISA_ALWAYS, // XorGdEdZeroIdiom
    ISA_ALWAYS, // SubEwGwZeroIdiom
    ISA_ALWAYS, // SubGwEwZeroIdiom
    ISA_ALWAYS, // SubEdGdZeroIdiom
    ISA_ALWAYS, // SubGdEdZeroIdiom
    ISA_ALWAYS, // AddGbEb
    ISA_ALWAYS, // OrGbEb
    ISA_ALWAYS, // AdcGbEb
    ISA_ALWAYS, // SbbGbEb
    ISA_ALWAYS, // AndGbEb
    ISA_ALWAYS, // SubGbEb
    ISA_ALWAYS, // XorGbEb
    ISA_ALWAYS, // CmpGbEb
    ISA_ALWAYS, // AdcGwEw
    ISA_ALWAYS, // AddGwEw
    ISA_ALWAYS, // AndGwEw
    ISA_ALWAYS, // CmpGwEw
    ISA_ALWAYS, // OrGwEw
    ISA_ALWAYS, // SbbGwEw
    ISA_ALWAYS, // SubGwEw
    ISA_ALWAYS, // XorGwEw
    ISA_ALWAYS, // AdcGdEd
    ISA_ALWAYS, // AddGdEd
    ISA_ALWAYS, // AndGdEd
    ISA_ALWAYS, // CmpGdEd
    ISA_ALWAYS, // OrGdEd
    ISA_ALWAYS, // SbbGdEd
    ISA_ALWAYS, // SubGdEd
    ISA_ALWAYS, // XorGdEd
    ISA_ALWAYS, // IncEb
    ISA_ALWAYS, // IncEw
    ISA_ALWAYS, // IncEd
    ISA_ALWAYS, // DecEb
    ISA_ALWAYS, // DecEw
    ISA_ALWAYS, // DecEd
    ISA_ALWAYS, // BsfGwEw
    ISA_ALWAYS, // BsrGwEw
    ISA_ALWAYS, // BsfGdEd
    ISA_ALWAYS, // BsrGdEd
    ISA_ALWAYS, // BtcEwGw
    ISA_ALWAYS, // BtrEwGw
    ISA_ALWAYS, // BtsEwGw
    ISA_ALWAYS, // BtcEdGd
    ISA_ALWAYS, // BtrEdGd
    ISA_ALWAYS, // BtsEdGd
    ISA_ALWAYS, // BtcEwIb
    ISA_ALWAYS, // BtrEwIb
    ISA_ALWAYS, // BtsEwIb
    ISA_ALWAYS, // BtcEdIb
    ISA_ALWAYS, // BtrEdIb
    ISA_ALWAYS, // BtsEdIb
    ISA_ALWAYS, // BtEwIb
    ISA_ALWAYS, // BtEdIb
    ISA_ALWAYS, // BtEwGw
    ISA_ALWAYS, // BtEdGd
    ISA_ALWAYS, // BoundGwMa
    ISA_ALWAYS, // BoundGdMa
    ISA_ALWAYS, // ArplEwGw
    ISA_ALWAYS, // CallEd
    ISA_ALWAYS, // CallEw
    ISA_ALWAYS, // CallJd
    ISA_ALWAYS, // CallJw
    ISA_ALWAYS, // CallfOp16Ap
    ISA_ALWAYS, // CallfOp32Ap
    ISA_ALWAYS, // CallfOp16Ep
    ISA_ALWAYS, // CallfOp32Ep
    ISA_ALWAYS, // Cbw
    ISA_ALWAYS, // Cdq
    ISA_ALWAYS, // Cwd
    ISA_ALWAYS, // Cwde
    ISA_ALWAYS, // Clc
    ISA_ALWAYS, // Cld
    ISA_ALWAYS, // Cli
    ISA_ALWAYS, // Clts
    ISA_ALWAYS, // Cmc
    ISA_ALWAYS, // Hlt
    17, // Clflush -> X86Feature::IsaClflush
    18, // Clflushopt -> X86Feature::IsaClflushopt
    19, // Clwb -> X86Feature::IsaClwb
    117, // Clzero -> X86Feature::IsaClzero
    ISA_ALWAYS, // EnterOp16IwIb
    ISA_ALWAYS, // EnterOp32IwIb
    ISA_ALWAYS, // LeaveOp16
    ISA_ALWAYS, // LeaveOp32
    ISA_ALWAYS, // ImulGdEd
    ISA_ALWAYS, // ImulGdEdId
    ISA_ALWAYS, // ImulGdEdsIb
    ISA_ALWAYS, // ImulGwEw
    ISA_ALWAYS, // ImulGwEwIw
    ISA_ALWAYS, // ImulGwEwsIb
    ISA_ALWAYS, // InAlDx
    ISA_ALWAYS, // InAlib
    ISA_ALWAYS, // InAxDx
    ISA_ALWAYS, // InAxib
    ISA_ALWAYS, // InEaxDx
    ISA_ALWAYS, // InEaxib
    ISA_ALWAYS, // OutDxAl
    ISA_ALWAYS, // OutDxAx
    ISA_ALWAYS, // OutDxEax
    ISA_ALWAYS, // OutIbAl
    ISA_ALWAYS, // OutIbAx
    ISA_ALWAYS, // OutIbEax
    ISA_ALWAYS, // IntIb
    ISA_ALWAYS, // INT1
    ISA_ALWAYS, // INT3
    ISA_ALWAYS, // Int0
    ISA_ALWAYS, // IretOp16
    ISA_ALWAYS, // IretOp32
    ISA_ALWAYS, // JmpEd
    ISA_ALWAYS, // JmpEw
    ISA_ALWAYS, // JmpJw
    ISA_ALWAYS, // JmpJbw
    ISA_ALWAYS, // JmpJd
    ISA_ALWAYS, // JmpJbd
    ISA_ALWAYS, // JmpfAp
    ISA_ALWAYS, // JmpfOp16Ep
    ISA_ALWAYS, // JmpfOp32Ep
    ISA_ALWAYS, // JcxzJbw
    ISA_ALWAYS, // JecxzJbd
    ISA_ALWAYS, // LoopJbw
    ISA_ALWAYS, // LoopeJbw
    ISA_ALWAYS, // LoopneJbw
    ISA_ALWAYS, // LoopJbd
    ISA_ALWAYS, // LoopeJbd
    ISA_ALWAYS, // LoopneJbd
    ISA_ALWAYS, // JbJw
    ISA_ALWAYS, // JbeJw
    ISA_ALWAYS, // JlJw
    ISA_ALWAYS, // JleJw
    ISA_ALWAYS, // JnbJw
    ISA_ALWAYS, // JnbeJw
    ISA_ALWAYS, // JnlJw
    ISA_ALWAYS, // JnleJw
    ISA_ALWAYS, // JnoJw
    ISA_ALWAYS, // JnpJw
    ISA_ALWAYS, // JnsJw
    ISA_ALWAYS, // JnzJw
    ISA_ALWAYS, // JoJw
    ISA_ALWAYS, // JpJw
    ISA_ALWAYS, // JsJw
    ISA_ALWAYS, // JzJw
    ISA_ALWAYS, // JbJbw
    ISA_ALWAYS, // JbeJbw
    ISA_ALWAYS, // JlJbw
    ISA_ALWAYS, // JleJbw
    ISA_ALWAYS, // JnbJbw
    ISA_ALWAYS, // JnbeJbw
    ISA_ALWAYS, // JnlJbw
    ISA_ALWAYS, // JnleJbw
    ISA_ALWAYS, // JnoJbw
    ISA_ALWAYS, // JnpJbw
    ISA_ALWAYS, // JnsJbw
    ISA_ALWAYS, // JnzJbw
    ISA_ALWAYS, // JoJbw
    ISA_ALWAYS, // JpJbw
    ISA_ALWAYS, // JsJbw
    ISA_ALWAYS, // JzJbw
    ISA_ALWAYS, // JbJd
    ISA_ALWAYS, // JbeJd
    ISA_ALWAYS, // JlJd
    ISA_ALWAYS, // JleJd
    ISA_ALWAYS, // JnbJd
    ISA_ALWAYS, // JnbeJd
    ISA_ALWAYS, // JnlJd
    ISA_ALWAYS, // JnleJd
    ISA_ALWAYS, // JnoJd
    ISA_ALWAYS, // JnpJd
    ISA_ALWAYS, // JnsJd
    ISA_ALWAYS, // JnzJd
    ISA_ALWAYS, // JoJd
    ISA_ALWAYS, // JpJd
    ISA_ALWAYS, // JsJd
    ISA_ALWAYS, // JzJd
    ISA_ALWAYS, // JbJbd
    ISA_ALWAYS, // JbeJbd
    ISA_ALWAYS, // JlJbd
    ISA_ALWAYS, // JleJbd
    ISA_ALWAYS, // JnbJbd
    ISA_ALWAYS, // JnbeJbd
    ISA_ALWAYS, // JnlJbd
    ISA_ALWAYS, // JnleJbd
    ISA_ALWAYS, // JnoJbd
    ISA_ALWAYS, // JnpJbd
    ISA_ALWAYS, // JnsJbd
    ISA_ALWAYS, // JnzJbd
    ISA_ALWAYS, // JoJbd
    ISA_ALWAYS, // JpJbd
    ISA_ALWAYS, // JsJbd
    ISA_ALWAYS, // JzJbd
    ISA_ALWAYS, // Sahf
    ISA_ALWAYS, // Lahf
    ISA_ALWAYS, // LdsGdMp
    ISA_ALWAYS, // LdsGwMp
    ISA_ALWAYS, // LesGdMp
    ISA_ALWAYS, // LesGwMp
    ISA_ALWAYS, // LfsGdMp
    ISA_ALWAYS, // LfsGwMp
    ISA_ALWAYS, // LssGdMp
    ISA_ALWAYS, // LssGwMp
    ISA_ALWAYS, // LgsGdMp
    ISA_ALWAYS, // LgsGwMp
    ISA_ALWAYS, // LarGwEw
    ISA_ALWAYS, // LslGwEw
    ISA_ALWAYS, // LarGdEw
    ISA_ALWAYS, // LslGdEw
    ISA_ALWAYS, // LeaGdM
    ISA_ALWAYS, // LeaGwM
    ISA_ALWAYS, // SidtMs
    ISA_ALWAYS, // LidtMs
    ISA_ALWAYS, // SgdtMs
    ISA_ALWAYS, // LgdtMs
    ISA_ALWAYS, // SldtEw
    ISA_ALWAYS, // LldtEw
    ISA_ALWAYS, // StrEw
    ISA_ALWAYS, // LtrEw
    ISA_ALWAYS, // SmswEw
    ISA_ALWAYS, // LmswEw
    ISA_ALWAYS, // MovCr0rd
    ISA_ALWAYS, // MovCr2rd
    ISA_ALWAYS, // MovCr3rd
    3, // MovCr4rd -> X86Feature::IsaPentium
    ISA_ALWAYS, // MovRdCr0
    ISA_ALWAYS, // MovRdCr2
    ISA_ALWAYS, // MovRdCr3
    3, // MovRdCr4 -> X86Feature::IsaPentium
    ISA_ALWAYS, // MovRdDd
    ISA_ALWAYS, // MovDdRd
    ISA_ALWAYS, // MovEbIb
    ISA_ALWAYS, // MovEdId
    ISA_ALWAYS, // MovEwIw
    ISA_ALWAYS, // MovGbEb
    ISA_ALWAYS, // MovEbGb
    ISA_ALWAYS, // MovGwEw
    ISA_ALWAYS, // MovEwGw
    ISA_ALWAYS, // MovOp32GdEd
    ISA_ALWAYS, // MovOp32EdGd
    ISA_ALWAYS, // MovEwSw
    ISA_ALWAYS, // MovSwEw
    ISA_ALWAYS, // MovAlod
    ISA_ALWAYS, // MovAxod
    ISA_ALWAYS, // MovEaxod
    ISA_ALWAYS, // MovOdAl
    ISA_ALWAYS, // MovOdAx
    ISA_ALWAYS, // MovOdEax
    ISA_ALWAYS, // MovsxGdEb
    ISA_ALWAYS, // MovsxGdEw
    ISA_ALWAYS, // MovsxGwEb
    ISA_ALWAYS, // MovzxGdEb
    ISA_ALWAYS, // MovzxGdEw
    ISA_ALWAYS, // MovzxGwEb
    ISA_ALWAYS, // Nop
    ISA_ALWAYS, // Pause
    ISA_ALWAYS, // PopEw
    ISA_ALWAYS, // PopEd
    ISA_ALWAYS, // PopOp16Sw
    ISA_ALWAYS, // PopOp32Sw
    ISA_ALWAYS, // PopaOp16
    ISA_ALWAYS, // PopaOp32
    ISA_ALWAYS, // PopfFw
    ISA_ALWAYS, // PopfFd
    ISA_ALWAYS, // PushEw
    ISA_ALWAYS, // PushEd
    ISA_ALWAYS, // PushId
    ISA_ALWAYS, // PushSIb32
    ISA_ALWAYS, // PushIw
    ISA_ALWAYS, // PushSIb16
    ISA_ALWAYS, // PushOp16Sw
    ISA_ALWAYS, // PushOp32Sw
    ISA_ALWAYS, // PushaOp16
    ISA_ALWAYS, // PushaOp32
    ISA_ALWAYS, // PushfFw
    ISA_ALWAYS, // PushfFd
    ISA_ALWAYS, // RepCmpsbXbYb
    ISA_ALWAYS, // RepCmpsdXdYd
    ISA_ALWAYS, // RepCmpswXwYw
    ISA_ALWAYS, // RepInsbYbDx
    ISA_ALWAYS, // RepInsdYdDx
    ISA_ALWAYS, // RepInswYwDx
    ISA_ALWAYS, // RepLodsbAlxb
    ISA_ALWAYS, // RepLodsdEaxxd
    ISA_ALWAYS, // RepLodswAxxw
    ISA_ALWAYS, // RepMovsbYbXb
    ISA_ALWAYS, // RepMovsdYdXd
    ISA_ALWAYS, // RepMovswYwXw
    ISA_ALWAYS, // RepOutsbDxxb
    ISA_ALWAYS, // RepOutsdDxxd
    ISA_ALWAYS, // RepOutswDxxw
    ISA_ALWAYS, // RepScasbAlyb
    ISA_ALWAYS, // RepScasdEaxyd
    ISA_ALWAYS, // RepScaswAxyw
    ISA_ALWAYS, // RepStosbYbAl
    ISA_ALWAYS, // RepStosdYdEax
    ISA_ALWAYS, // RepStoswYwAx
    ISA_ALWAYS, // RetfOp16
    ISA_ALWAYS, // RetfOp16Iw
    ISA_ALWAYS, // RetfOp32
    ISA_ALWAYS, // RetfOp32Iw
    ISA_ALWAYS, // RetOp16
    ISA_ALWAYS, // RetOp16Iw
    ISA_ALWAYS, // RetOp32
    ISA_ALWAYS, // RetOp32Iw
    ISA_ALWAYS, // NotEb
    ISA_ALWAYS, // NegEb
    ISA_ALWAYS, // NotEw
    ISA_ALWAYS, // NegEw
    ISA_ALWAYS, // NotEd
    ISA_ALWAYS, // NegEd
    ISA_ALWAYS, // RolEb
    ISA_ALWAYS, // RorEb
    ISA_ALWAYS, // RclEb
    ISA_ALWAYS, // RcrEb
    ISA_ALWAYS, // ShlEb
    ISA_ALWAYS, // ShrEb
    ISA_ALWAYS, // SarEb
    ISA_ALWAYS, // RolEw
    ISA_ALWAYS, // RorEw
    ISA_ALWAYS, // RclEw
    ISA_ALWAYS, // RcrEw
    ISA_ALWAYS, // ShlEw
    ISA_ALWAYS, // ShrEw
    ISA_ALWAYS, // SarEw
    ISA_ALWAYS, // RolEd
    ISA_ALWAYS, // RorEd
    ISA_ALWAYS, // RclEd
    ISA_ALWAYS, // RcrEd
    ISA_ALWAYS, // ShlEd
    ISA_ALWAYS, // ShrEd
    ISA_ALWAYS, // SarEd
    ISA_ALWAYS, // RolEbIb
    ISA_ALWAYS, // RorEbIb
    ISA_ALWAYS, // RclEbIb
    ISA_ALWAYS, // RcrEbIb
    ISA_ALWAYS, // ShlEbIb
    ISA_ALWAYS, // ShrEbIb
    ISA_ALWAYS, // SarEbIb
    ISA_ALWAYS, // RolEwIb
    ISA_ALWAYS, // RorEwIb
    ISA_ALWAYS, // RclEwIb
    ISA_ALWAYS, // RcrEwIb
    ISA_ALWAYS, // ShlEwIb
    ISA_ALWAYS, // ShrEwIb
    ISA_ALWAYS, // SarEwIb
    ISA_ALWAYS, // RolEdIb
    ISA_ALWAYS, // RorEdIb
    ISA_ALWAYS, // RclEdIb
    ISA_ALWAYS, // RcrEdIb
    ISA_ALWAYS, // ShlEdIb
    ISA_ALWAYS, // ShrEdIb
    ISA_ALWAYS, // SarEdIb
    ISA_ALWAYS, // RolEbI1
    ISA_ALWAYS, // RorEbI1
    ISA_ALWAYS, // RclEbI1
    ISA_ALWAYS, // RcrEbI1
    ISA_ALWAYS, // ShlEbI1
    ISA_ALWAYS, // ShrEbI1
    ISA_ALWAYS, // SarEbI1
    ISA_ALWAYS, // RolEwI1
    ISA_ALWAYS, // RorEwI1
    ISA_ALWAYS, // RclEwI1
    ISA_ALWAYS, // RcrEwI1
    ISA_ALWAYS, // ShlEwI1
    ISA_ALWAYS, // ShrEwI1
    ISA_ALWAYS, // SarEwI1
    ISA_ALWAYS, // RolEdI1
    ISA_ALWAYS, // RorEdI1
    ISA_ALWAYS, // RclEdI1
    ISA_ALWAYS, // RcrEdI1
    ISA_ALWAYS, // ShlEdI1
    ISA_ALWAYS, // ShrEdI1
    ISA_ALWAYS, // SarEdI1
    ISA_ALWAYS, // SetbEb
    ISA_ALWAYS, // SetbeEb
    ISA_ALWAYS, // SetlEb
    ISA_ALWAYS, // SetleEb
    ISA_ALWAYS, // SetnbEb
    ISA_ALWAYS, // SetnbeEb
    ISA_ALWAYS, // SetnlEb
    ISA_ALWAYS, // SetnleEb
    ISA_ALWAYS, // SetnoEb
    ISA_ALWAYS, // SetnpEb
    ISA_ALWAYS, // SetnsEb
    ISA_ALWAYS, // SetnzEb
    ISA_ALWAYS, // SetoEb
    ISA_ALWAYS, // SetpEb
    ISA_ALWAYS, // SetsEb
    ISA_ALWAYS, // SetzEb
    ISA_ALWAYS, // ShldEdGd
    ISA_ALWAYS, // ShldEdGdIb
    ISA_ALWAYS, // ShldEwGw
    ISA_ALWAYS, // ShldEwGwIb
    ISA_ALWAYS, // ShrdEdGd
    ISA_ALWAYS, // ShrdEdGdIb
    ISA_ALWAYS, // ShrdEwGw
    ISA_ALWAYS, // ShrdEwGwIb
    ISA_ALWAYS, // Rsm
    ISA_ALWAYS, // Salc
    ISA_ALWAYS, // Stc
    ISA_ALWAYS, // Std
    ISA_ALWAYS, // Sti
    ISA_ALWAYS, // MulAleb
    ISA_ALWAYS, // ImulAleb
    ISA_ALWAYS, // DivAleb
    ISA_ALWAYS, // IdivAleb
    ISA_ALWAYS, // MulAxew
    ISA_ALWAYS, // ImulAxew
    ISA_ALWAYS, // DivAxew
    ISA_ALWAYS, // IdivAxew
    ISA_ALWAYS, // MulEaxed
    ISA_ALWAYS, // ImulEaxed
    ISA_ALWAYS, // DivEaxed
    ISA_ALWAYS, // IdivEaxed
    ISA_ALWAYS, // VerrEw
    ISA_ALWAYS, // VerwEw
    ISA_ALWAYS, // XchgEbGb
    ISA_ALWAYS, // XchgEwGw
    ISA_ALWAYS, // XchgEdGd
    ISA_ALWAYS, // XchgRxax
    ISA_ALWAYS, // XchgErxEax
    ISA_ALWAYS, // Xlat
    16, // Sysenter -> X86Feature::IsaSysenterSysexit
    16, // Sysexit -> X86Feature::IsaSysenterSysexit
    27, // Monitor -> X86Feature::IsaMonitorMwait
    27, // Mwait -> X86Feature::IsaMonitorMwait
    28, // UmonitorEq -> X86Feature::IsaWaitpkg
    28, // UmonitorEd -> X86Feature::IsaWaitpkg
    28, // UmwaitEd -> X86Feature::IsaWaitpkg
    28, // TpauseEd -> X86Feature::IsaWaitpkg
    30, // Monitorx -> X86Feature::IsaMonitorxMwaitx
    30, // Mwaitx -> X86Feature::IsaMonitorxMwaitx
    1, // Fwait -> X86Feature::IsaX87
    1, // FldSti -> X86Feature::IsaX87
    1, // FldSingleReal -> X86Feature::IsaX87
    1, // FldDoubleReal -> X86Feature::IsaX87
    1, // FldExtendedReal -> X86Feature::IsaX87
    1, // FildWordInteger -> X86Feature::IsaX87
    1, // FildDwordInteger -> X86Feature::IsaX87
    1, // FildQwordInteger -> X86Feature::IsaX87
    1, // FbldPackedBcd -> X86Feature::IsaX87
    1, // FstSti -> X86Feature::IsaX87
    1, // FstpSti -> X86Feature::IsaX87
    1, // FstpSpecialSti -> X86Feature::IsaX87
    1, // FstSingleReal -> X86Feature::IsaX87
    1, // FstpSingleReal -> X86Feature::IsaX87
    1, // FstDoubleReal -> X86Feature::IsaX87
    1, // FstpDoubleReal -> X86Feature::IsaX87
    1, // FstpExtendedReal -> X86Feature::IsaX87
    1, // FistWordInteger -> X86Feature::IsaX87
    1, // FistpWordInteger -> X86Feature::IsaX87
    1, // FistDwordInteger -> X86Feature::IsaX87
    1, // FistpDwordInteger -> X86Feature::IsaX87
    1, // FistpQwordInteger -> X86Feature::IsaX87
    1, // FbstpPackedBcd -> X86Feature::IsaX87
    22, // FisttpMw -> X86Feature::IsaSse3
    22, // FisttpMd -> X86Feature::IsaSse3
    22, // FisttpMq -> X86Feature::IsaSse3
    1, // Fninit -> X86Feature::IsaX87
    1, // Fnclex -> X86Feature::IsaX87
    1, // Frstor -> X86Feature::IsaX87
    1, // Fnsave -> X86Feature::IsaX87
    1, // Fldenv -> X86Feature::IsaX87
    1, // Fnstenv -> X86Feature::IsaX87
    1, // Fldcw -> X86Feature::IsaX87
    1, // Fnstcw -> X86Feature::IsaX87
    1, // Fnstsw -> X86Feature::IsaX87
    1, // FnstswAx -> X86Feature::IsaX87
    1, // FLD1 -> X86Feature::IsaX87
    1, // Fldl2t -> X86Feature::IsaX87
    1, // Fldl2e -> X86Feature::IsaX87
    1, // Fldpi -> X86Feature::IsaX87
    1, // Fldlg2 -> X86Feature::IsaX87
    1, // Fldln2 -> X86Feature::IsaX87
    1, // Fldz -> X86Feature::IsaX87
    1, // FaddSt0Stj -> X86Feature::IsaX87
    1, // FaddStiSt0 -> X86Feature::IsaX87
    1, // FaddpStiSt0 -> X86Feature::IsaX87
    1, // FaddSingleReal -> X86Feature::IsaX87
    1, // FaddDoubleReal -> X86Feature::IsaX87
    1, // FiaddWordInteger -> X86Feature::IsaX87
    1, // FiaddDwordInteger -> X86Feature::IsaX87
    1, // FmulSt0Stj -> X86Feature::IsaX87
    1, // FmulStiSt0 -> X86Feature::IsaX87
    1, // FmulpStiSt0 -> X86Feature::IsaX87
    1, // FmulSingleReal -> X86Feature::IsaX87
    1, // FmulDoubleReal -> X86Feature::IsaX87
    1, // FimulWordInteger -> X86Feature::IsaX87
    1, // FimulDwordInteger -> X86Feature::IsaX87
    1, // FsubSt0Stj -> X86Feature::IsaX87
    1, // FsubrSt0Stj -> X86Feature::IsaX87
    1, // FsubStiSt0 -> X86Feature::IsaX87
    1, // FsubpStiSt0 -> X86Feature::IsaX87
    1, // FsubrStiSt0 -> X86Feature::IsaX87
    1, // FsubrpStiSt0 -> X86Feature::IsaX87
    1, // FsubSingleReal -> X86Feature::IsaX87
    1, // FsubrSingleReal -> X86Feature::IsaX87
    1, // FsubDoubleReal -> X86Feature::IsaX87
    1, // FsubrDoubleReal -> X86Feature::IsaX87
    1, // FisubWordInteger -> X86Feature::IsaX87
    1, // FisubrWordInteger -> X86Feature::IsaX87
    1, // FisubDwordInteger -> X86Feature::IsaX87
    1, // FisubrDwordInteger -> X86Feature::IsaX87
    1, // FdivSt0Stj -> X86Feature::IsaX87
    1, // FdivrSt0Stj -> X86Feature::IsaX87
    1, // FdivStiSt0 -> X86Feature::IsaX87
    1, // FdivpStiSt0 -> X86Feature::IsaX87
    1, // FdivrStiSt0 -> X86Feature::IsaX87
    1, // FdivrpStiSt0 -> X86Feature::IsaX87
    1, // FdivSingleReal -> X86Feature::IsaX87
    1, // FdivrSingleReal -> X86Feature::IsaX87
    1, // FdivDoubleReal -> X86Feature::IsaX87
    1, // FdivrDoubleReal -> X86Feature::IsaX87
    1, // FidivWordInteger -> X86Feature::IsaX87
    1, // FidivrWordInteger -> X86Feature::IsaX87
    1, // FidivDwordInteger -> X86Feature::IsaX87
    1, // FidivrDwordInteger -> X86Feature::IsaX87
    1, // FcomSti -> X86Feature::IsaX87
    1, // FcompSti -> X86Feature::IsaX87
    1, // FucomSti -> X86Feature::IsaX87
    1, // FucompSti -> X86Feature::IsaX87
    4, // FcomiSt0Stj -> X86Feature::IsaP6
    4, // FcomipSt0Stj -> X86Feature::IsaP6
    4, // FucomiSt0Stj -> X86Feature::IsaP6
    4, // FucomipSt0Stj -> X86Feature::IsaP6
    1, // FcomSingleReal -> X86Feature::IsaX87
    1, // FcompSingleReal -> X86Feature::IsaX87
    1, // FcomDoubleReal -> X86Feature::IsaX87
    1, // FcompDoubleReal -> X86Feature::IsaX87
    1, // FicomWordInteger -> X86Feature::IsaX87
    1, // FicompWordInteger -> X86Feature::IsaX87
    1, // FicomDwordInteger -> X86Feature::IsaX87
    1, // FicompDwordInteger -> X86Feature::IsaX87
    4, // FcmovbSt0Stj -> X86Feature::IsaP6
    4, // FcmoveSt0Stj -> X86Feature::IsaP6
    4, // FcmovbeSt0Stj -> X86Feature::IsaP6
    4, // FcmovuSt0Stj -> X86Feature::IsaP6
    4, // FcmovnbSt0Stj -> X86Feature::IsaP6
    4, // FcmovneSt0Stj -> X86Feature::IsaP6
    4, // FcmovnbeSt0Stj -> X86Feature::IsaP6
    4, // FcmovnuSt0Stj -> X86Feature::IsaP6
    1, // Fcompp -> X86Feature::IsaX87
    1, // Fucompp -> X86Feature::IsaX87
    1, // FxchSti -> X86Feature::IsaX87
    1, // Fnop -> X86Feature::IsaX87
    1, // Fplegacy -> X86Feature::IsaX87
    1, // Fchs -> X86Feature::IsaX87
    1, // Fabs -> X86Feature::IsaX87
    1, // Ftst -> X86Feature::IsaX87
    1, // Fxam -> X86Feature::IsaX87
    1, // Fdecstp -> X86Feature::IsaX87
    1, // Fincstp -> X86Feature::IsaX87
    1, // FfreeSti -> X86Feature::IsaX87
    1, // FfreepSti -> X86Feature::IsaX87
    1, // F2XM1 -> X86Feature::IsaX87
    1, // FYL2X -> X86Feature::IsaX87
    1, // Fptan -> X86Feature::IsaX87
    1, // Fpatan -> X86Feature::IsaX87
    1, // Fxtract -> X86Feature::IsaX87
    1, // FPREM1 -> X86Feature::IsaX87
    1, // Fprem -> X86Feature::IsaX87
    1, // FYL2XP1 -> X86Feature::IsaX87
    1, // Fsqrt -> X86Feature::IsaX87
    1, // Fsincos -> X86Feature::IsaX87
    1, // Frndint -> X86Feature::IsaX87
    1, // Fscale -> X86Feature::IsaX87
    1, // Fsin -> X86Feature::IsaX87
    1, // Fcos -> X86Feature::IsaX87
    ISA_ALWAYS, // Fpuesc
    2, // Cpuid -> X86Feature::Isa486
    2, // BswapRx -> X86Feature::Isa486
    2, // BswapErx -> X86Feature::Isa486
    2, // Invd -> X86Feature::Isa486
    2, // Wbinvd -> X86Feature::Isa486
    2, // XaddEbGb -> X86Feature::Isa486
    2, // XaddEwGw -> X86Feature::Isa486
    2, // XaddEdGd -> X86Feature::Isa486
    2, // CmpxchgEbGb -> X86Feature::Isa486
    2, // CmpxchgEwGw -> X86Feature::Isa486
    2, // CmpxchgEdGd -> X86Feature::Isa486
    ISA_ALWAYS, // Invlpg
    3, // Cmpxchg8b -> X86Feature::IsaPentium
    3, // Wrmsr -> X86Feature::IsaPentium
    3, // Rdmsr -> X86Feature::IsaPentium
    3, // Rdtsc -> X86Feature::IsaPentium
    5, // PunpcklbwPqQd -> X86Feature::IsaMmx
    5, // PunpcklwdPqQd -> X86Feature::IsaMmx
    5, // PunpckldqPqQd -> X86Feature::IsaMmx
    5, // PacksswbPqQq -> X86Feature::IsaMmx
    5, // PcmpgtbPqQq -> X86Feature::IsaMmx
    5, // PcmpgtwPqQq -> X86Feature::IsaMmx
    5, // PcmpgtdPqQq -> X86Feature::IsaMmx
    5, // PackuswbPqQq -> X86Feature::IsaMmx
    5, // PunpckhbwPqQq -> X86Feature::IsaMmx
    5, // PunpckhwdPqQq -> X86Feature::IsaMmx
    5, // PunpckhdqPqQq -> X86Feature::IsaMmx
    5, // PackssdwPqQq -> X86Feature::IsaMmx
    5, // MovdPqEd -> X86Feature::IsaMmx
    5, // MovqPqQq -> X86Feature::IsaMmx
    5, // PcmpeqbPqQq -> X86Feature::IsaMmx
    5, // PcmpeqwPqQq -> X86Feature::IsaMmx
    5, // PcmpeqdPqQq -> X86Feature::IsaMmx
    5, // Emms -> X86Feature::IsaMmx
    5, // MovdEdPq -> X86Feature::IsaMmx
    5, // MovqQqPq -> X86Feature::IsaMmx
    5, // PsrlwPqQq -> X86Feature::IsaMmx
    5, // PsrldPqQq -> X86Feature::IsaMmx
    5, // PsrlqPqQq -> X86Feature::IsaMmx
    5, // PmullwPqQq -> X86Feature::IsaMmx
    5, // PsubusbPqQq -> X86Feature::IsaMmx
    5, // PsubuswPqQq -> X86Feature::IsaMmx
    5, // PandPqQq -> X86Feature::IsaMmx
    5, // PaddusbPqQq -> X86Feature::IsaMmx
    5, // PadduswPqQq -> X86Feature::IsaMmx
    5, // PandnPqQq -> X86Feature::IsaMmx
    5, // PsrawPqQq -> X86Feature::IsaMmx
    5, // PsradPqQq -> X86Feature::IsaMmx
    5, // PmulhwPqQq -> X86Feature::IsaMmx
    5, // PsubsbPqQq -> X86Feature::IsaMmx
    5, // PsubswPqQq -> X86Feature::IsaMmx
    5, // PorPqQq -> X86Feature::IsaMmx
    5, // PaddsbPqQq -> X86Feature::IsaMmx
    5, // PaddswPqQq -> X86Feature::IsaMmx
    5, // PxorPqQq -> X86Feature::IsaMmx
    5, // PsllwPqQq -> X86Feature::IsaMmx
    5, // PslldPqQq -> X86Feature::IsaMmx
    5, // PsllqPqQq -> X86Feature::IsaMmx
    5, // PmaddwdPqQq -> X86Feature::IsaMmx
    5, // PsubbPqQq -> X86Feature::IsaMmx
    5, // PsubwPqQq -> X86Feature::IsaMmx
    5, // PsubdPqQq -> X86Feature::IsaMmx
    5, // PaddbPqQq -> X86Feature::IsaMmx
    5, // PaddwPqQq -> X86Feature::IsaMmx
    5, // PadddPqQq -> X86Feature::IsaMmx
    5, // PsrlwNqIb -> X86Feature::IsaMmx
    5, // PsrawNqIb -> X86Feature::IsaMmx
    5, // PsllwNqIb -> X86Feature::IsaMmx
    5, // PsrldNqIb -> X86Feature::IsaMmx
    5, // PsradNqIb -> X86Feature::IsaMmx
    5, // PslldNqIb -> X86Feature::IsaMmx
    5, // PsrlqNqIb -> X86Feature::IsaMmx
    5, // PsllqNqIb -> X86Feature::IsaMmx
    ISA_ALWAYS, // MovqEqPq
    6, // Femms -> X86Feature::Isa3dnow
    6, // Pf2idPqQq -> X86Feature::Isa3dnow
    7, // Pf2iwPqQq -> X86Feature::Isa3dnowExt
    6, // PfaccPqQq -> X86Feature::Isa3dnow
    6, // PfaddPqQq -> X86Feature::Isa3dnow
    6, // PfcmpeqPqQq -> X86Feature::Isa3dnow
    6, // PfcmpgePqQq -> X86Feature::Isa3dnow
    6, // PfcmpgtPqQq -> X86Feature::Isa3dnow
    6, // PfmaxPqQq -> X86Feature::Isa3dnow
    6, // PfminPqQq -> X86Feature::Isa3dnow
    6, // PfmulPqQq -> X86Feature::Isa3dnow
    7, // PfnaccPqQq -> X86Feature::Isa3dnowExt
    7, // PfpnaccPqQq -> X86Feature::Isa3dnowExt
    6, // PfrcpPqQq -> X86Feature::Isa3dnow
    6, // Pfrcpit1PqQq -> X86Feature::Isa3dnow
    6, // Pfrcpit2PqQq -> X86Feature::Isa3dnow
    6, // Pfrsqit1PqQq -> X86Feature::Isa3dnow
    6, // PfrsqrtPqQq -> X86Feature::Isa3dnow
    6, // PfsubPqQq -> X86Feature::Isa3dnow
    6, // PfsubrPqQq -> X86Feature::Isa3dnow
    6, // Pi2fdPqQq -> X86Feature::Isa3dnow
    7, // Pi2fwPqQq -> X86Feature::Isa3dnowExt
    6, // PmulhrwPqQq -> X86Feature::Isa3dnow
    7, // PswapdPqQq -> X86Feature::Isa3dnowExt
    ISA_ALWAYS, // PrefetchwMb
    15, // SyscallLegacy -> X86Feature::IsaSyscallSysretLegacy
    15, // SysretLegacy -> X86Feature::IsaSyscallSysretLegacy
    4, // CmovbGwEw -> X86Feature::IsaP6
    4, // CmovbeGwEw -> X86Feature::IsaP6
    4, // CmovlGwEw -> X86Feature::IsaP6
    4, // CmovleGwEw -> X86Feature::IsaP6
    4, // CmovnbGwEw -> X86Feature::IsaP6
    4, // CmovnbeGwEw -> X86Feature::IsaP6
    4, // CmovnlGwEw -> X86Feature::IsaP6
    4, // CmovnleGwEw -> X86Feature::IsaP6
    4, // CmovnoGwEw -> X86Feature::IsaP6
    4, // CmovnpGwEw -> X86Feature::IsaP6
    4, // CmovnsGwEw -> X86Feature::IsaP6
    4, // CmovnzGwEw -> X86Feature::IsaP6
    4, // CmovoGwEw -> X86Feature::IsaP6
    4, // CmovpGwEw -> X86Feature::IsaP6
    4, // CmovsGwEw -> X86Feature::IsaP6
    4, // CmovzGwEw -> X86Feature::IsaP6
    4, // CmovbGdEd -> X86Feature::IsaP6
    4, // CmovbeGdEd -> X86Feature::IsaP6
    4, // CmovlGdEd -> X86Feature::IsaP6
    4, // CmovleGdEd -> X86Feature::IsaP6
    4, // CmovnbGdEd -> X86Feature::IsaP6
    4, // CmovnbeGdEd -> X86Feature::IsaP6
    4, // CmovnlGdEd -> X86Feature::IsaP6
    4, // CmovnleGdEd -> X86Feature::IsaP6
    4, // CmovnoGdEd -> X86Feature::IsaP6
    4, // CmovnpGdEd -> X86Feature::IsaP6
    4, // CmovnsGdEd -> X86Feature::IsaP6
    4, // CmovnzGdEd -> X86Feature::IsaP6
    4, // CmovoGdEd -> X86Feature::IsaP6
    4, // CmovpGdEd -> X86Feature::IsaP6
    4, // CmovsGdEd -> X86Feature::IsaP6
    4, // CmovzGdEd -> X86Feature::IsaP6
    4, // Rdpmc -> X86Feature::IsaP6
    ISA_ALWAYS, // Ud0
    ISA_ALWAYS, // Ud1
    ISA_ALWAYS, // Ud2
    20, // Fxsave -> X86Feature::IsaSse
    20, // Fxrstor -> X86Feature::IsaSse
    20, // Ldmxcsr -> X86Feature::IsaSse
    20, // Stmxcsr -> X86Feature::IsaSse
    20, // PrefetchMb -> X86Feature::IsaSse
    20, // Prefetcht0Mb -> X86Feature::IsaSse
    20, // Prefetcht1Mb -> X86Feature::IsaSse
    20, // Prefetcht2Mb -> X86Feature::IsaSse
    20, // PrefetchntaMb -> X86Feature::IsaSse
    20, // AndpsVpsWps -> X86Feature::IsaSse
    20, // OrpsVpsWps -> X86Feature::IsaSse
    20, // XorpsVpsWps -> X86Feature::IsaSse
    20, // AndnpsVpsWps -> X86Feature::IsaSse
    20, // MovupsVpsWps -> X86Feature::IsaSse
    20, // MovupsWpsVps -> X86Feature::IsaSse
    20, // MovssVssWss -> X86Feature::IsaSse
    20, // MovssWssVss -> X86Feature::IsaSse
    20, // MovlpsVpsMq -> X86Feature::IsaSse
    20, // MovhlpsVpsWps -> X86Feature::IsaSse
    20, // MovlpsMqVps -> X86Feature::IsaSse
    20, // MovhpsVpsMq -> X86Feature::IsaSse
    20, // MovlhpsVpsWps -> X86Feature::IsaSse
    20, // MovhpsMqVps -> X86Feature::IsaSse
    20, // MovapsVpsWps -> X86Feature::IsaSse
    20, // MovapsWpsVps -> X86Feature::IsaSse
    20, // MovntpsMpsVps -> X86Feature::IsaSse
    20, // Cvtpi2psVpsQq -> X86Feature::IsaSse
    20, // Cvtsi2ssVssEd -> X86Feature::IsaSse
    20, // Cvttps2piPqWps -> X86Feature::IsaSse
    20, // Cvtps2piPqWps -> X86Feature::IsaSse
    20, // Cvttss2siGdWss -> X86Feature::IsaSse
    20, // Cvtss2siGdWss -> X86Feature::IsaSse
    20, // UcomissVssWss -> X86Feature::IsaSse
    20, // ComissVssWss -> X86Feature::IsaSse
    20, // MovmskpsGdUps -> X86Feature::IsaSse
    21, // MovmskpdGdUpd -> X86Feature::IsaSse2
    20, // RsqrtpsVpsWps -> X86Feature::IsaSse
    20, // RsqrtssVssWss -> X86Feature::IsaSse
    20, // RcppsVpsWps -> X86Feature::IsaSse
    20, // RcpssVssWss -> X86Feature::IsaSse
    20, // PshufwPqQqIb -> X86Feature::IsaSse
    20, // PshuflwVdqWdqIb -> X86Feature::IsaSse
    20, // PinsrwPqEwIb -> X86Feature::IsaSse
    20, // PextrwGdNqIb -> X86Feature::IsaSse
    20, // ShufpsVpsWpsIb -> X86Feature::IsaSse
    20, // PmovmskbGdNq -> X86Feature::IsaSse
    20, // PminubPqQq -> X86Feature::IsaSse
    20, // PmaxubPqQq -> X86Feature::IsaSse
    20, // PavgbPqQq -> X86Feature::IsaSse
    20, // PavgwPqQq -> X86Feature::IsaSse
    20, // PmulhuwPqQq -> X86Feature::IsaSse
    20, // MovntqMqPq -> X86Feature::IsaSse
    20, // PminswPqQq -> X86Feature::IsaSse
    20, // PmaxswPqQq -> X86Feature::IsaSse
    20, // PsadbwPqQq -> X86Feature::IsaSse
    20, // MaskmovqPqNq -> X86Feature::IsaSse
    20, // AddpsVpsWps -> X86Feature::IsaSse
    21, // AddpdVpdWpd -> X86Feature::IsaSse2
    20, // AddssVssWss -> X86Feature::IsaSse
    21, // AddsdVsdWsd -> X86Feature::IsaSse2
    20, // MulpsVpsWps -> X86Feature::IsaSse
    21, // MulpdVpdWpd -> X86Feature::IsaSse2
    20, // MulssVssWss -> X86Feature::IsaSse
    21, // MulsdVsdWsd -> X86Feature::IsaSse2
    20, // SubpsVpsWps -> X86Feature::IsaSse
    21, // SubpdVpdWpd -> X86Feature::IsaSse2
    20, // SubssVssWss -> X86Feature::IsaSse
    21, // SubsdVsdWsd -> X86Feature::IsaSse2
    20, // MinpsVpsWps -> X86Feature::IsaSse
    21, // MinpdVpdWpd -> X86Feature::IsaSse2
    20, // MinssVssWss -> X86Feature::IsaSse
    21, // MinsdVsdWsd -> X86Feature::IsaSse2
    20, // DivpsVpsWps -> X86Feature::IsaSse
    21, // DivpdVpdWpd -> X86Feature::IsaSse2
    20, // DivssVssWss -> X86Feature::IsaSse
    21, // DivsdVsdWsd -> X86Feature::IsaSse2
    20, // MaxpsVpsWps -> X86Feature::IsaSse
    21, // MaxpdVpdWpd -> X86Feature::IsaSse2
    20, // MaxssVssWss -> X86Feature::IsaSse
    21, // MaxsdVsdWsd -> X86Feature::IsaSse2
    20, // SqrtpsVpsWps -> X86Feature::IsaSse
    21, // SqrtpdVpdWpd -> X86Feature::IsaSse2
    20, // SqrtssVssWss -> X86Feature::IsaSse
    21, // SqrtsdVsdWsd -> X86Feature::IsaSse2
    20, // CmppsVpsWpsIb -> X86Feature::IsaSse
    21, // CmppdVpdWpdIb -> X86Feature::IsaSse2
    20, // CmpssVssWssIb -> X86Feature::IsaSse
    21, // CmpsdVsdWsdIb -> X86Feature::IsaSse2
    21, // Cvtps2pdVpdWps -> X86Feature::IsaSse2
    21, // Cvtpd2psVpsWpd -> X86Feature::IsaSse2
    21, // Cvtss2sdVsdWss -> X86Feature::IsaSse2
    21, // Cvtsd2ssVssWsd -> X86Feature::IsaSse2
    21, // MovsdVsdWsd -> X86Feature::IsaSse2
    21, // MovsdWsdVsd -> X86Feature::IsaSse2
    21, // Cvtpi2pdVpdQq -> X86Feature::IsaSse2
    21, // Cvtsi2sdVsdEd -> X86Feature::IsaSse2
    21, // Cvttpd2piPqWpd -> X86Feature::IsaSse2
    21, // Cvttsd2siGdWsd -> X86Feature::IsaSse2
    21, // Cvtpd2piPqWpd -> X86Feature::IsaSse2
    21, // Cvtsd2siGdWsd -> X86Feature::IsaSse2
    21, // UcomisdVsdWsd -> X86Feature::IsaSse2
    21, // ComisdVsdWsd -> X86Feature::IsaSse2
    21, // Cvtdq2psVpsWdq -> X86Feature::IsaSse2
    21, // Cvtps2dqVdqWps -> X86Feature::IsaSse2
    21, // Cvttps2dqVdqWps -> X86Feature::IsaSse2
    21, // UnpckhpdVpdWdq -> X86Feature::IsaSse2
    21, // UnpcklpdVpdWdq -> X86Feature::IsaSse2
    21, // PunpckhdqVdqWdq -> X86Feature::IsaSse2
    21, // PunpckldqVdqWdq -> X86Feature::IsaSse2
    21, // MovapdVpdWpd -> X86Feature::IsaSse2
    21, // MovapdWpdVpd -> X86Feature::IsaSse2
    21, // MovdqaVdqWdq -> X86Feature::IsaSse2
    21, // MovdqaWdqVdq -> X86Feature::IsaSse2
    21, // MovdquVdqWdq -> X86Feature::IsaSse2
    21, // MovdquWdqVdq -> X86Feature::IsaSse2
    21, // MovhpdMqVsd -> X86Feature::IsaSse2
    21, // MovhpdVsdMq -> X86Feature::IsaSse2
    21, // MovlpdMqVsd -> X86Feature::IsaSse2
    21, // MovlpdVsdMq -> X86Feature::IsaSse2
    21, // MovntdqMdqVdq -> X86Feature::IsaSse2
    21, // MovntpdMpdVpd -> X86Feature::IsaSse2
    21, // MovupdVpdWpd -> X86Feature::IsaSse2
    21, // MovupdWpdVpd -> X86Feature::IsaSse2
    21, // AndnpdVpdWpd -> X86Feature::IsaSse2
    21, // AndpdVpdWpd -> X86Feature::IsaSse2
    21, // OrpdVpdWpd -> X86Feature::IsaSse2
    21, // XorpdVpdWpd -> X86Feature::IsaSse2
    21, // PandVdqWdq -> X86Feature::IsaSse2
    21, // PandnVdqWdq -> X86Feature::IsaSse2
    21, // PorVdqWdq -> X86Feature::IsaSse2
    21, // PxorVdqWdq -> X86Feature::IsaSse2
    21, // PunpcklbwVdqWdq -> X86Feature::IsaSse2
    21, // PunpcklwdVdqWdq -> X86Feature::IsaSse2
    20, // UnpcklpsVpsWdq -> X86Feature::IsaSse
    20, // UnpckhpsVpsWdq -> X86Feature::IsaSse
    21, // PackuswbVdqWdq -> X86Feature::IsaSse2
    21, // PacksswbVdqWdq -> X86Feature::IsaSse2
    21, // PcmpgtbVdqWdq -> X86Feature::IsaSse2
    21, // PcmpgtwVdqWdq -> X86Feature::IsaSse2
    21, // PcmpgtdVdqWdq -> X86Feature::IsaSse2
    21, // PunpckhbwVdqWdq -> X86Feature::IsaSse2
    21, // PunpckhwdVdqWdq -> X86Feature::IsaSse2
    21, // PackssdwVdqWdq -> X86Feature::IsaSse2
    21, // PunpcklqdqVdqWdq -> X86Feature::IsaSse2
    21, // PunpckhqdqVdqWdq -> X86Feature::IsaSse2
    21, // MovdVdqEd -> X86Feature::IsaSse2
    21, // PshufdVdqWdqIb -> X86Feature::IsaSse2
    21, // PshufhwVdqWdqIb -> X86Feature::IsaSse2
    21, // PcmpeqbVdqWdq -> X86Feature::IsaSse2
    21, // PcmpeqwVdqWdq -> X86Feature::IsaSse2
    21, // PcmpeqdVdqWdq -> X86Feature::IsaSse2
    21, // MovdEdVd -> X86Feature::IsaSse2
    21, // MovqVqWq -> X86Feature::IsaSse2
    21, // MovntiOp32MdGd -> X86Feature::IsaSse2
    21, // PinsrwVdqEwIb -> X86Feature::IsaSse2
    21, // PextrwGdUdqIb -> X86Feature::IsaSse2
    21, // ShufpdVpdWpdIb -> X86Feature::IsaSse2
    21, // PsrlwVdqWdq -> X86Feature::IsaSse2
    21, // PsrldVdqWdq -> X86Feature::IsaSse2
    21, // PsrlqVdqWdq -> X86Feature::IsaSse2
    21, // PaddqPqQq -> X86Feature::IsaSse2
    21, // PsubqPqQq -> X86Feature::IsaSse2
    21, // PaddqVdqWdq -> X86Feature::IsaSse2
    21, // PmullwVdqWdq -> X86Feature::IsaSse2
    21, // MovqWqVq -> X86Feature::IsaSse2
    21, // Movdq2qPqUdq -> X86Feature::IsaSse2
    21, // Movq2dqVdqQq -> X86Feature::IsaSse2
    21, // PmovmskbGdUdq -> X86Feature::IsaSse2
    21, // PsubusbVdqWdq -> X86Feature::IsaSse2
    21, // PsubuswVdqWdq -> X86Feature::IsaSse2
    21, // PminubVdqWdq -> X86Feature::IsaSse2
    21, // PaddusbVdqWdq -> X86Feature::IsaSse2
    21, // PadduswVdqWdq -> X86Feature::IsaSse2
    21, // PmaxubVdqWdq -> X86Feature::IsaSse2
    21, // PavgbVdqWdq -> X86Feature::IsaSse2
    21, // PsrawVdqWdq -> X86Feature::IsaSse2
    21, // PsradVdqWdq -> X86Feature::IsaSse2
    21, // PavgwVdqWdq -> X86Feature::IsaSse2
    21, // PmulhuwVdqWdq -> X86Feature::IsaSse2
    21, // PmulhwVdqWdq -> X86Feature::IsaSse2
    21, // Cvttpd2dqVqWpd -> X86Feature::IsaSse2
    21, // Cvtpd2dqVqWpd -> X86Feature::IsaSse2
    21, // Cvtdq2pdVpdWq -> X86Feature::IsaSse2
    21, // PsubsbVdqWdq -> X86Feature::IsaSse2
    21, // PsubswVdqWdq -> X86Feature::IsaSse2
    21, // PminswVdqWdq -> X86Feature::IsaSse2
    21, // PmaxswVdqWdq -> X86Feature::IsaSse2
    21, // PaddsbVdqWdq -> X86Feature::IsaSse2
    21, // PaddswVdqWdq -> X86Feature::IsaSse2
    21, // PsllwVdqWdq -> X86Feature::IsaSse2
    21, // PslldVdqWdq -> X86Feature::IsaSse2
    21, // PsllqVdqWdq -> X86Feature::IsaSse2
    21, // PmuludqPqQq -> X86Feature::IsaSse2
    21, // PmuludqVdqWdq -> X86Feature::IsaSse2
    21, // PmaddwdVdqWdq -> X86Feature::IsaSse2
    21, // PsadbwVdqWdq -> X86Feature::IsaSse2
    21, // MaskmovdquVdqUdq -> X86Feature::IsaSse2
    21, // PsubbVdqWdq -> X86Feature::IsaSse2
    21, // PsubwVdqWdq -> X86Feature::IsaSse2
    21, // PsubdVdqWdq -> X86Feature::IsaSse2
    21, // PsubqVdqWdq -> X86Feature::IsaSse2
    21, // PaddbVdqWdq -> X86Feature::IsaSse2
    21, // PaddwVdqWdq -> X86Feature::IsaSse2
    21, // PadddVdqWdq -> X86Feature::IsaSse2
    21, // PsrlwUdqIb -> X86Feature::IsaSse2
    21, // PsrawUdqIb -> X86Feature::IsaSse2
    21, // PsllwUdqIb -> X86Feature::IsaSse2
    21, // PsrldUdqIb -> X86Feature::IsaSse2
    21, // PsradUdqIb -> X86Feature::IsaSse2
    21, // PslldUdqIb -> X86Feature::IsaSse2
    21, // PsrlqUdqIb -> X86Feature::IsaSse2
    21, // PsllqUdqIb -> X86Feature::IsaSse2
    21, // PsrldqUdqIb -> X86Feature::IsaSse2
    21, // PslldqUdqIb -> X86Feature::IsaSse2
    21, // Lfence -> X86Feature::IsaSse2
    20, // Sfence -> X86Feature::IsaSse
    21, // Mfence -> X86Feature::IsaSse2
    22, // MovddupVpdWq -> X86Feature::IsaSse3
    22, // MovsldupVpsWps -> X86Feature::IsaSse3
    22, // MovshdupVpsWps -> X86Feature::IsaSse3
    22, // HaddpdVpdWpd -> X86Feature::IsaSse3
    22, // HaddpsVpsWps -> X86Feature::IsaSse3
    22, // HsubpdVpdWpd -> X86Feature::IsaSse3
    22, // HsubpsVpsWps -> X86Feature::IsaSse3
    22, // AddsubpdVpdWpd -> X86Feature::IsaSse3
    22, // AddsubpsVpsWps -> X86Feature::IsaSse3
    22, // LddquVdqMdq -> X86Feature::IsaSse3
    23, // PshufbPqQq -> X86Feature::IsaSsse3
    23, // PhaddwPqQq -> X86Feature::IsaSsse3
    23, // PhadddPqQq -> X86Feature::IsaSsse3
    23, // PhaddswPqQq -> X86Feature::IsaSsse3
    23, // PmaddubswPqQq -> X86Feature::IsaSsse3
    23, // PhsubswPqQq -> X86Feature::IsaSsse3
    23, // PhsubwPqQq -> X86Feature::IsaSsse3
    23, // PhsubdPqQq -> X86Feature::IsaSsse3
    23, // PsignbPqQq -> X86Feature::IsaSsse3
    23, // PsignwPqQq -> X86Feature::IsaSsse3
    23, // PsigndPqQq -> X86Feature::IsaSsse3
    23, // PmulhrswPqQq -> X86Feature::IsaSsse3
    23, // PabsbPqQq -> X86Feature::IsaSsse3
    23, // PabswPqQq -> X86Feature::IsaSsse3
    23, // PabsdPqQq -> X86Feature::IsaSsse3
    23, // PalignrPqQqIb -> X86Feature::IsaSsse3
    23, // PshufbVdqWdq -> X86Feature::IsaSsse3
    23, // PhaddwVdqWdq -> X86Feature::IsaSsse3
    23, // PhadddVdqWdq -> X86Feature::IsaSsse3
    23, // PhaddswVdqWdq -> X86Feature::IsaSsse3
    23, // PmaddubswVdqWdq -> X86Feature::IsaSsse3
    23, // PhsubswVdqWdq -> X86Feature::IsaSsse3
    23, // PhsubwVdqWdq -> X86Feature::IsaSsse3
    23, // PhsubdVdqWdq -> X86Feature::IsaSsse3
    23, // PsignbVdqWdq -> X86Feature::IsaSsse3
    23, // PsignwVdqWdq -> X86Feature::IsaSsse3
    23, // PsigndVdqWdq -> X86Feature::IsaSsse3
    23, // PmulhrswVdqWdq -> X86Feature::IsaSsse3
    23, // PabsbVdqWdq -> X86Feature::IsaSsse3
    23, // PabswVdqWdq -> X86Feature::IsaSsse3
    23, // PabsdVdqWdq -> X86Feature::IsaSsse3
    23, // PalignrVdqWdqIb -> X86Feature::IsaSsse3
    24, // PblendvbVdqWdq -> X86Feature::IsaSse4_1
    24, // BlendvpsVpsWps -> X86Feature::IsaSse4_1
    24, // BlendvpdVpdWpd -> X86Feature::IsaSse4_1
    24, // PmovsxbwVdqWq -> X86Feature::IsaSse4_1
    24, // PmovsxbdVdqWd -> X86Feature::IsaSse4_1
    24, // PmovsxbqVdqWw -> X86Feature::IsaSse4_1
    24, // PmovsxwdVdqWq -> X86Feature::IsaSse4_1
    24, // PmovsxwqVdqWd -> X86Feature::IsaSse4_1
    24, // PmovsxdqVdqWq -> X86Feature::IsaSse4_1
    24, // PmovzxbwVdqWq -> X86Feature::IsaSse4_1
    24, // PmovzxbdVdqWd -> X86Feature::IsaSse4_1
    24, // PmovzxbqVdqWw -> X86Feature::IsaSse4_1
    24, // PmovzxwdVdqWq -> X86Feature::IsaSse4_1
    24, // PmovzxwqVdqWd -> X86Feature::IsaSse4_1
    24, // PmovzxdqVdqWq -> X86Feature::IsaSse4_1
    24, // PtestVdqWdq -> X86Feature::IsaSse4_1
    24, // PmuldqVdqWdq -> X86Feature::IsaSse4_1
    24, // PcmpeqqVdqWdq -> X86Feature::IsaSse4_1
    24, // PackusdwVdqWdq -> X86Feature::IsaSse4_1
    24, // PminsbVdqWdq -> X86Feature::IsaSse4_1
    24, // PminsdVdqWdq -> X86Feature::IsaSse4_1
    24, // PminuwVdqWdq -> X86Feature::IsaSse4_1
    24, // PminudVdqWdq -> X86Feature::IsaSse4_1
    24, // PmaxsbVdqWdq -> X86Feature::IsaSse4_1
    24, // PmaxsdVdqWdq -> X86Feature::IsaSse4_1
    24, // PmaxuwVdqWdq -> X86Feature::IsaSse4_1
    24, // PmaxudVdqWdq -> X86Feature::IsaSse4_1
    24, // PmulldVdqWdq -> X86Feature::IsaSse4_1
    24, // PhminposuwVdqWdq -> X86Feature::IsaSse4_1
    24, // RoundpsVpsWpsIb -> X86Feature::IsaSse4_1
    24, // RoundpdVpdWpdIb -> X86Feature::IsaSse4_1
    24, // RoundssVssWssIb -> X86Feature::IsaSse4_1
    24, // RoundsdVsdWsdIb -> X86Feature::IsaSse4_1
    24, // BlendpsVpsWpsIb -> X86Feature::IsaSse4_1
    24, // BlendpdVpdWpdIb -> X86Feature::IsaSse4_1
    24, // PblendwVdqWdqIb -> X86Feature::IsaSse4_1
    24, // PextrbEdVdqIbR -> X86Feature::IsaSse4_1
    24, // PextrbMbVdqIbM -> X86Feature::IsaSse4_1
    24, // PextrwEdVdqIbR -> X86Feature::IsaSse4_1
    24, // PextrwMwVdqIbM -> X86Feature::IsaSse4_1
    24, // PextrdEdVdqIb -> X86Feature::IsaSse4_1
    24, // PextrqEqVdqIb -> X86Feature::IsaSse4_1
    24, // ExtractpsEdVpsIb -> X86Feature::IsaSse4_1
    24, // PinsrbVdqEbIb -> X86Feature::IsaSse4_1
    24, // InsertpsVpsWssIb -> X86Feature::IsaSse4_1
    24, // PinsrdVdqEdIb -> X86Feature::IsaSse4_1
    24, // PinsrqVdqEqIb -> X86Feature::IsaSse4_1
    24, // DppsVpsWpsIb -> X86Feature::IsaSse4_1
    24, // DppdVpdWpdIb -> X86Feature::IsaSse4_1
    24, // MpsadbwVdqWdqIb -> X86Feature::IsaSse4_1
    24, // MovntdqaVdqMdq -> X86Feature::IsaSse4_1
    25, // Crc32GdEb -> X86Feature::IsaSse4_2
    25, // Crc32GdEw -> X86Feature::IsaSse4_2
    25, // Crc32GdEd -> X86Feature::IsaSse4_2
    25, // Crc32GdEq -> X86Feature::IsaSse4_2
    25, // PcmpgtqVdqWdq -> X86Feature::IsaSse4_2
    25, // PcmpestrmVdqWdqIb -> X86Feature::IsaSse4_2
    25, // PcmpestriVdqWdqIb -> X86Feature::IsaSse4_2
    25, // PcmpistrmVdqWdqIb -> X86Feature::IsaSse4_2
    25, // PcmpistriVdqWdqIb -> X86Feature::IsaSse4_2
    44, // MovbeGwMw -> X86Feature::IsaMovbe
    44, // MovbeGdMd -> X86Feature::IsaMovbe
    44, // MovbeGqMq -> X86Feature::IsaMovbe
    44, // MovbeMwGw -> X86Feature::IsaMovbe
    44, // MovbeMdGd -> X86Feature::IsaMovbe
    44, // MovbeMqGq -> X86Feature::IsaMovbe
    25, // PopcntGwEw -> X86Feature::IsaSse4_2
    25, // PopcntGdEd -> X86Feature::IsaSse4_2
    25, // PopcntGqEq -> X86Feature::IsaSse4_2
    38, // Xrstor -> X86Feature::IsaXsave
    38, // Xsave -> X86Feature::IsaXsave
    40, // Xsavec -> X86Feature::IsaXsavec
    38, // Xsetbv -> X86Feature::IsaXsave
    38, // Xgetbv -> X86Feature::IsaXsave
    39, // Xsaveopt -> X86Feature::IsaXsaveopt
    41, // Xsaves -> X86Feature::IsaXsaves
    41, // Xrstors -> X86Feature::IsaXsaves
    42, // AesimcVdqWdq -> X86Feature::IsaAesPclmulqdq
    42, // AeskeygenassistVdqWdqIb -> X86Feature::IsaAesPclmulqdq
    42, // AesencVdqWdq -> X86Feature::IsaAesPclmulqdq
    42, // AesenclastVdqWdq -> X86Feature::IsaAesPclmulqdq
    42, // AesdecVdqWdq -> X86Feature::IsaAesPclmulqdq
    42, // AesdeclastVdqWdq -> X86Feature::IsaAesPclmulqdq
    42, // PclmulqdqVdqWdqIb -> X86Feature::IsaAesPclmulqdq
    67, // Sha1nexteVdqWdq -> X86Feature::IsaSha
    67, // Sha1msg1VdqWdq -> X86Feature::IsaSha
    67, // Sha1msg2VdqWdq -> X86Feature::IsaSha
    67, // Sha256rnds2VdqWdq -> X86Feature::IsaSha
    67, // Sha256msg1VdqWdq -> X86Feature::IsaSha
    67, // Sha256msg2VdqWdq -> X86Feature::IsaSha
    67, // Sha1rnds4VdqWdqIb -> X86Feature::IsaSha
    69, // Gf2p8affineqbVdqWdqIb -> X86Feature::IsaGfni
    69, // Gf2p8affineinvqbVdqWdqIb -> X86Feature::IsaGfni
    69, // Gf2p8mulbVdqWdq -> X86Feature::IsaGfni
    32, // LahfLm -> X86Feature::IsaLmLahfSahf
    32, // SahfLm -> X86Feature::IsaLmLahfSahf
    ISA_ALWAYS, // Syscall
    ISA_ALWAYS, // Sysret
    ISA_ALWAYS, // XorEqGqZeroIdiom
    ISA_ALWAYS, // XorGqEqZeroIdiom
    ISA_ALWAYS, // SubEqGqZeroIdiom
    ISA_ALWAYS, // SubGqEqZeroIdiom
    ISA_ALWAYS, // AddGqEq
    ISA_ALWAYS, // OrGqEq
    ISA_ALWAYS, // AdcGqEq
    ISA_ALWAYS, // SbbGqEq
    ISA_ALWAYS, // AndGqEq
    ISA_ALWAYS, // SubGqEq
    ISA_ALWAYS, // XorGqEq
    ISA_ALWAYS, // CmpGqEq
    ISA_ALWAYS, // AddEqGq
    ISA_ALWAYS, // OrEqGq
    ISA_ALWAYS, // AdcEqGq
    ISA_ALWAYS, // SbbEqGq
    ISA_ALWAYS, // AndEqGq
    ISA_ALWAYS, // SubEqGq
    ISA_ALWAYS, // XorEqGq
    ISA_ALWAYS, // TestEqGq
    ISA_ALWAYS, // CmpEqGq
    ISA_ALWAYS, // AddRaxid
    ISA_ALWAYS, // OrRaxid
    ISA_ALWAYS, // AdcRaxid
    ISA_ALWAYS, // SbbRaxid
    ISA_ALWAYS, // AndRaxid
    ISA_ALWAYS, // SubRaxid
    ISA_ALWAYS, // XorRaxid
    ISA_ALWAYS, // TestRaxid
    ISA_ALWAYS, // CmpRaxid
    ISA_ALWAYS, // AddEqId
    ISA_ALWAYS, // OrEqId
    ISA_ALWAYS, // AdcEqId
    ISA_ALWAYS, // SbbEqId
    ISA_ALWAYS, // AndEqId
    ISA_ALWAYS, // SubEqId
    ISA_ALWAYS, // XorEqId
    ISA_ALWAYS, // TestEqId
    ISA_ALWAYS, // CmpEqId
    ISA_ALWAYS, // AddEqsIb
    ISA_ALWAYS, // OrEqsIb
    ISA_ALWAYS, // AdcEqsIb
    ISA_ALWAYS, // SbbEqsIb
    ISA_ALWAYS, // AndEqsIb
    ISA_ALWAYS, // SubEqsIb
    ISA_ALWAYS, // XorEqsIb
    ISA_ALWAYS, // TestEqsIb
    ISA_ALWAYS, // CmpEqsIb
    ISA_ALWAYS, // XchgEqGq
    ISA_ALWAYS, // XchgRrxRax
    ISA_ALWAYS, // LeaGqM
    ISA_ALWAYS, // MovOp64GdEd
    ISA_ALWAYS, // MovOp64EdGd
    ISA_ALWAYS, // MovGqEq
    ISA_ALWAYS, // MovEqGq
    ISA_ALWAYS, // MovEqId
    ISA_ALWAYS, // MovRaxoq
    ISA_ALWAYS, // MovOqRax
    ISA_ALWAYS, // MovEaxoq
    ISA_ALWAYS, // MovOqEax
    ISA_ALWAYS, // MovAxoq
    ISA_ALWAYS, // MovOqAx
    ISA_ALWAYS, // MovAloq
    ISA_ALWAYS, // MovOqAl
    ISA_ALWAYS, // RepMovsqYqXq
    ISA_ALWAYS, // RepCmpsqXqYq
    ISA_ALWAYS, // RepStosqYqRax
    ISA_ALWAYS, // RepLodsqRaxxq
    ISA_ALWAYS, // RepScasqRaxyq
    ISA_ALWAYS, // CallJq
    ISA_ALWAYS, // JmpJq
    ISA_ALWAYS, // JmpJbq
    ISA_ALWAYS, // JoJq
    ISA_ALWAYS, // JnoJq
    ISA_ALWAYS, // JbJq
    ISA_ALWAYS, // JnbJq
    ISA_ALWAYS, // JzJq
    ISA_ALWAYS, // JnzJq
    ISA_ALWAYS, // JbeJq
    ISA_ALWAYS, // JnbeJq
    ISA_ALWAYS, // JsJq
    ISA_ALWAYS, // JnsJq
    ISA_ALWAYS, // JpJq
    ISA_ALWAYS, // JnpJq
    ISA_ALWAYS, // JlJq
    ISA_ALWAYS, // JnlJq
    ISA_ALWAYS, // JleJq
    ISA_ALWAYS, // JnleJq
    ISA_ALWAYS, // JoJbq
    ISA_ALWAYS, // JnoJbq
    ISA_ALWAYS, // JbJbq
    ISA_ALWAYS, // JnbJbq
    ISA_ALWAYS, // JzJbq
    ISA_ALWAYS, // JnzJbq
    ISA_ALWAYS, // JbeJbq
    ISA_ALWAYS, // JnbeJbq
    ISA_ALWAYS, // JsJbq
    ISA_ALWAYS, // JnsJbq
    ISA_ALWAYS, // JpJbq
    ISA_ALWAYS, // JnpJbq
    ISA_ALWAYS, // JlJbq
    ISA_ALWAYS, // JnlJbq
    ISA_ALWAYS, // JleJbq
    ISA_ALWAYS, // JnleJbq
    ISA_ALWAYS, // EnterOp64IwIb
    ISA_ALWAYS, // LeaveOp64
    ISA_ALWAYS, // IretOp64
    ISA_ALWAYS, // ShldEqGq
    ISA_ALWAYS, // ShldEqGqIb
    ISA_ALWAYS, // ShrdEqGq
    ISA_ALWAYS, // ShrdEqGqIb
    ISA_ALWAYS, // ImulGqEq
    ISA_ALWAYS, // ImulGqEqId
    ISA_ALWAYS, // ImulGqEqsIb
    ISA_ALWAYS, // MovzxGqEb
    ISA_ALWAYS, // MovzxGqEw
    ISA_ALWAYS, // MovsxGqEb
    ISA_ALWAYS, // MovsxGqEw
    ISA_ALWAYS, // MovsxdGqEd
    ISA_ALWAYS, // BswapRrx
    ISA_ALWAYS, // BsfGqEq
    ISA_ALWAYS, // BsrGqEq
    ISA_ALWAYS, // BtEqGq
    ISA_ALWAYS, // BtsEqGq
    ISA_ALWAYS, // BtrEqGq
    ISA_ALWAYS, // BtcEqGq
    ISA_ALWAYS, // BtEqIb
    ISA_ALWAYS, // BtsEqIb
    ISA_ALWAYS, // BtrEqIb
    ISA_ALWAYS, // BtcEqIb
    ISA_ALWAYS, // NotEq
    ISA_ALWAYS, // NegEq
    ISA_ALWAYS, // RolEq
    ISA_ALWAYS, // RorEq
    ISA_ALWAYS, // RclEq
    ISA_ALWAYS, // RcrEq
    ISA_ALWAYS, // ShlEq
    ISA_ALWAYS, // ShrEq
    ISA_ALWAYS, // SarEq
    ISA_ALWAYS, // RolEqIb
    ISA_ALWAYS, // RorEqIb
    ISA_ALWAYS, // RclEqIb
    ISA_ALWAYS, // RcrEqIb
    ISA_ALWAYS, // ShlEqIb
    ISA_ALWAYS, // ShrEqIb
    ISA_ALWAYS, // SarEqIb
    ISA_ALWAYS, // RolEqI1
    ISA_ALWAYS, // RorEqI1
    ISA_ALWAYS, // RclEqI1
    ISA_ALWAYS, // RcrEqI1
    ISA_ALWAYS, // ShlEqI1
    ISA_ALWAYS, // ShrEqI1
    ISA_ALWAYS, // SarEqI1
    ISA_ALWAYS, // MulRaxeq
    ISA_ALWAYS, // ImulRaxeq
    ISA_ALWAYS, // DivRaxeq
    ISA_ALWAYS, // IdivRaxeq
    ISA_ALWAYS, // IncEq
    ISA_ALWAYS, // DecEq
    ISA_ALWAYS, // CallEq
    ISA_ALWAYS, // CallfOp64Ep
    ISA_ALWAYS, // JmpEq
    ISA_ALWAYS, // JmpfOp64Ep
    ISA_ALWAYS, // PushfFq
    ISA_ALWAYS, // PopfFq
    ISA_ALWAYS, // CmpxchgEqGq
    ISA_ALWAYS, // Cdqe
    ISA_ALWAYS, // Cqo
    ISA_ALWAYS, // XaddEqGq
    ISA_ALWAYS, // RetOp64Iw
    ISA_ALWAYS, // RetOp64
    ISA_ALWAYS, // RetfOp64Iw
    ISA_ALWAYS, // RetfOp64
    ISA_ALWAYS, // CmovoGqEq
    ISA_ALWAYS, // CmovnoGqEq
    ISA_ALWAYS, // CmovbGqEq
    ISA_ALWAYS, // CmovnbGqEq
    ISA_ALWAYS, // CmovzGqEq
    ISA_ALWAYS, // CmovnzGqEq
    ISA_ALWAYS, // CmovbeGqEq
    ISA_ALWAYS, // CmovnbeGqEq
    ISA_ALWAYS, // CmovsGqEq
    ISA_ALWAYS, // CmovnsGqEq
    ISA_ALWAYS, // CmovpGqEq
    ISA_ALWAYS, // CmovnpGqEq
    ISA_ALWAYS, // CmovlGqEq
    ISA_ALWAYS, // CmovnlGqEq
    ISA_ALWAYS, // CmovleGqEq
    ISA_ALWAYS, // CmovnleGqEq
    ISA_ALWAYS, // PushEq
    ISA_ALWAYS, // PopEq
    ISA_ALWAYS, // PushOp64Id
    ISA_ALWAYS, // PushOp64SIb
    ISA_ALWAYS, // PushOp64Sw
    ISA_ALWAYS, // PopOp64Sw
    ISA_ALWAYS, // SgdtOp64Ms
    ISA_ALWAYS, // SidtOp64Ms
    ISA_ALWAYS, // LgdtOp64Ms
    ISA_ALWAYS, // LidtOp64Ms
    ISA_ALWAYS, // MovRrxiq
    ISA_ALWAYS, // LssGqMp
    ISA_ALWAYS, // LfsGqMp
    ISA_ALWAYS, // LgsGqMp
    35, // CMPXCHG16B -> X86Feature::IsaCmpxchg16b
    ISA_ALWAYS, // LoopneJbq
    ISA_ALWAYS, // LoopeJbq
    ISA_ALWAYS, // LoopJbq
    ISA_ALWAYS, // JrcxzJbq
    ISA_ALWAYS, // MovqEqVq
    ISA_ALWAYS, // MovqPqEq
    ISA_ALWAYS, // MovqVdqEq
    ISA_ALWAYS, // Cvtsi2ssVssEq
    ISA_ALWAYS, // Cvtsi2sdVsdEq
    ISA_ALWAYS, // Cvttss2siGqWss
    ISA_ALWAYS, // Cvttsd2siGqWsd
    ISA_ALWAYS, // Cvtss2siGqWss
    ISA_ALWAYS, // Cvtsd2siGqWsd
    21, // MovntiOp64MdGd -> X86Feature::IsaSse2
    21, // MovntiMqGq -> X86Feature::IsaSse2
    ISA_ALWAYS, // MovCr0rq
    ISA_ALWAYS, // MovCr2rq
    ISA_ALWAYS, // MovCr3rq
    ISA_ALWAYS, // MovCr4rq
    ISA_ALWAYS, // MovRqCr0
    ISA_ALWAYS, // MovRqCr2
    ISA_ALWAYS, // MovRqCr3
    ISA_ALWAYS, // MovRqCr4
    ISA_ALWAYS, // MovDqRq
    ISA_ALWAYS, // MovRqDq
    ISA_ALWAYS, // Swapgs
    45, // RdfsbaseEd -> X86Feature::IsaFsgsbase
    45, // RdgsbaseEd -> X86Feature::IsaFsgsbase
    45, // RdfsbaseEq -> X86Feature::IsaFsgsbase
    45, // RdgsbaseEq -> X86Feature::IsaFsgsbase
    45, // WrfsbaseEd -> X86Feature::IsaFsgsbase
    45, // WrgsbaseEd -> X86Feature::IsaFsgsbase
    45, // WrfsbaseEq -> X86Feature::IsaFsgsbase
    45, // WrgsbaseEq -> X86Feature::IsaFsgsbase
    36, // Rdtscp -> X86Feature::IsaRdtscp
    60, // VmxonMq -> X86Feature::IsaVmx
    60, // Vmxoff -> X86Feature::IsaVmx
    60, // Vmcall -> X86Feature::IsaVmx
    60, // Vmlaunch -> X86Feature::IsaVmx
    60, // Vmresume -> X86Feature::IsaVmx
    60, // VmclearMq -> X86Feature::IsaVmx
    60, // VmptrldMq -> X86Feature::IsaVmx
    60, // VmptrstMq -> X86Feature::IsaVmx
    60, // VmreadEdGd -> X86Feature::IsaVmx
    60, // VmwriteGdEd -> X86Feature::IsaVmx
    60, // VmreadEqGq -> X86Feature::IsaVmx
    60, // VmwriteGqEq -> X86Feature::IsaVmx
    60, // Invept -> X86Feature::IsaVmx
    60, // Invvpid -> X86Feature::IsaVmx
    60, // Vmfunc -> X86Feature::IsaVmx
    61, // Getsec -> X86Feature::IsaSmx
    59, // Vmrun -> X86Feature::IsaSvm
    59, // Vmmcall -> X86Feature::IsaSvm
    59, // Vmload -> X86Feature::IsaSvm
    59, // Vmsave -> X86Feature::IsaSvm
    59, // Stgi -> X86Feature::IsaSvm
    59, // Clgi -> X86Feature::IsaSvm
    59, // Skinit -> X86Feature::IsaSvm
    59, // Invlpga -> X86Feature::IsaSvm
    119, // Incsspd -> X86Feature::IsaCet
    119, // Incsspq -> X86Feature::IsaCet
    ISA_ALWAYS, // Rdsspd
    ISA_ALWAYS, // Rdsspq
    119, // Saveprevssp -> X86Feature::IsaCet
    119, // Rstorssp -> X86Feature::IsaCet
    119, // Wrssd -> X86Feature::IsaCet
    119, // Wrussd -> X86Feature::IsaCet
    119, // Wrssq -> X86Feature::IsaCet
    119, // Wrussq -> X86Feature::IsaCet
    119, // Setssbsy -> X86Feature::IsaCet
    119, // Clrssbsy -> X86Feature::IsaCet
    ISA_ALWAYS, // Endbranch32
    ISA_ALWAYS, // Endbranch64
    106, // Invpcid -> X86Feature::IsaInvpcid
    112, // Rdpkru -> X86Feature::IsaPku
    112, // Wrpkru -> X86Feature::IsaPku
    126, // Clui -> X86Feature::IsaUintr
    126, // Stui -> X86Feature::IsaUintr
    126, // Testui -> X86Feature::IsaUintr
    126, // Uiret -> X86Feature::IsaUintr
    126, // SenduipiEq -> X86Feature::IsaUintr
    115, // RdpidEd -> X86Feature::IsaRdpid
    123, // Serialize -> X86Feature::IsaSerialize
    120, // Wrmsrns -> X86Feature::IsaWrmsrns
    130, // Rdmsrlist -> X86Feature::IsaMsrlist
    130, // Wrmsrlist -> X86Feature::IsaMsrlist
    46, // Vzeroupper -> X86Feature::IsaAvx
    46, // Vzeroall -> X86Feature::IsaAvx
    46, // Vldmxcsr -> X86Feature::IsaAvx
    46, // Vstmxcsr -> X86Feature::IsaAvx
    46, // VmovapsVpsWps -> X86Feature::IsaAvx
    46, // V128VmovapsWpsVps -> X86Feature::IsaAvx
    46, // V256VmovapsWpsVps -> X86Feature::IsaAvx
    46, // VmovapdVpdWpd -> X86Feature::IsaAvx
    46, // V128VmovapdWpdVpd -> X86Feature::IsaAvx
    46, // V256VmovapdWpdVpd -> X86Feature::IsaAvx
    46, // VmovupsVpsWps -> X86Feature::IsaAvx
    46, // V128VmovupsWpsVps -> X86Feature::IsaAvx
    46, // V256VmovupsWpsVps -> X86Feature::IsaAvx
    46, // VmovupdVpdWpd -> X86Feature::IsaAvx
    46, // V128VmovupdWpdVpd -> X86Feature::IsaAvx
    46, // V256VmovupdWpdVpd -> X86Feature::IsaAvx
    46, // VmovdqaVdqWdq -> X86Feature::IsaAvx
    46, // V128VmovdqaWdqVdq -> X86Feature::IsaAvx
    46, // V256VmovdqaWdqVdq -> X86Feature::IsaAvx
    46, // VmovdquVdqWdq -> X86Feature::IsaAvx
    46, // V128VmovdquWdqVdq -> X86Feature::IsaAvx
    46, // V256VmovdquWdqVdq -> X86Feature::IsaAvx
    46, // V128VmovsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // V128VmovssVssHpsWss -> X86Feature::IsaAvx
    46, // V128VmovsdWsdHpdVsd -> X86Feature::IsaAvx
    46, // V128VmovssWssHpsVss -> X86Feature::IsaAvx
    46, // V128VmovsdVsdWsd -> X86Feature::IsaAvx
    46, // V128VmovssVssWss -> X86Feature::IsaAvx
    46, // V128VmovsdWsdVsd -> X86Feature::IsaAvx
    46, // V128VmovssWssVss -> X86Feature::IsaAvx
    46, // V128VmovlpsVpsHpsMq -> X86Feature::IsaAvx
    46, // V128VmovhlpsVpsHpsWps -> X86Feature::IsaAvx
    46, // V128VmovhpsVpsHpsMq -> X86Feature::IsaAvx
    46, // V128VmovlhpsVpsHpsWps -> X86Feature::IsaAvx
    46, // V128VmovlpsMqVps -> X86Feature::IsaAvx
    46, // V128VmovhpsMqVps -> X86Feature::IsaAvx
    46, // V128VmovlpdMqVsd -> X86Feature::IsaAvx
    46, // V128VmovhpdMqVsd -> X86Feature::IsaAvx
    46, // V128VmovlpdVpdHpdMq -> X86Feature::IsaAvx
    46, // V128VmovhpdVpdHpdMq -> X86Feature::IsaAvx
    46, // V128VmovddupVpdWpd -> X86Feature::IsaAvx
    46, // V256VmovddupVpdWpd -> X86Feature::IsaAvx
    46, // VmovsldupVpsWps -> X86Feature::IsaAvx
    46, // VmovshdupVpsWps -> X86Feature::IsaAvx
    46, // VlddquVdqMdq -> X86Feature::IsaAvx
    46, // V128VmovntdqaVdqMdq -> X86Feature::IsaAvx
    47, // V256VmovntdqaVdqMdq -> X86Feature::IsaAvx2
    46, // V128VmovntpsMpsVps -> X86Feature::IsaAvx
    46, // V256VmovntpsMpsVps -> X86Feature::IsaAvx
    46, // V128VmovntpdMpdVpd -> X86Feature::IsaAvx
    46, // V256VmovntpdMpdVpd -> X86Feature::IsaAvx
    46, // V128VmovntdqMdqVdq -> X86Feature::IsaAvx
    46, // V256VmovntdqMdqVdq -> X86Feature::IsaAvx
    46, // VucomissVssWss -> X86Feature::IsaAvx
    46, // VcomissVssWss -> X86Feature::IsaAvx
    46, // VucomisdVsdWsd -> X86Feature::IsaAvx
    46, // VcomisdVsdWsd -> X86Feature::IsaAvx
    46, // VrsqrtssVssHpsWss -> X86Feature::IsaAvx
    46, // VrsqrtpsVpsWps -> X86Feature::IsaAvx
    46, // VrcpssVssHpsWss -> X86Feature::IsaAvx
    46, // VrcppsVpsWps -> X86Feature::IsaAvx
    46, // VandpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VandpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VandnpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VandnpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VorpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VorpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VxorpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VxorpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // V128VpshufdVdqWdqIb -> X86Feature::IsaAvx
    47, // V256VpshufdVdqWdqIb -> X86Feature::IsaAvx2
    46, // V128VpshufhwVdqWdqIb -> X86Feature::IsaAvx
    47, // V256VpshufhwVdqWdqIb -> X86Feature::IsaAvx2
    46, // V128VpshuflwVdqWdqIb -> X86Feature::IsaAvx
    47, // V256VpshuflwVdqWdqIb -> X86Feature::IsaAvx2
    46, // VhaddpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VhaddpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VhsubpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VhsubpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VshufpsVpsHpsWpsIb -> X86Feature::IsaAvx
    46, // VshufpdVpdHpdWpdIb -> X86Feature::IsaAvx
    46, // VaddsubpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VaddsubpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VroundpsVpsWpsIb -> X86Feature::IsaAvx
    46, // VroundpdVpdWpdIb -> X86Feature::IsaAvx
    46, // VroundsdVsdHpdWsdIb -> X86Feature::IsaAvx
    46, // VroundssVssHpsWssIb -> X86Feature::IsaAvx
    46, // VdppsVpsHpsWpsIb -> X86Feature::IsaAvx
    46, // VdppdVpdHpdWpdIb -> X86Feature::IsaAvx
    46, // VaddpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VaddpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VaddssVssHpsWss -> X86Feature::IsaAvx
    46, // VaddsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VmulpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VmulpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VmulssVssHpsWss -> X86Feature::IsaAvx
    46, // VmulsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VsubpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VsubpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VsubssVssHpsWss -> X86Feature::IsaAvx
    46, // VsubsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VdivpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VdivpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VdivssVssHpsWss -> X86Feature::IsaAvx
    46, // VdivsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VmaxpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VmaxpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VmaxssVssHpsWss -> X86Feature::IsaAvx
    46, // VmaxsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VminpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VminpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VminssVssHpsWss -> X86Feature::IsaAvx
    46, // VminsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VsqrtpsVpsWps -> X86Feature::IsaAvx
    46, // VsqrtpdVpdWpd -> X86Feature::IsaAvx
    46, // VsqrtssVssHpsWss -> X86Feature::IsaAvx
    46, // VsqrtsdVsdHpdWsd -> X86Feature::IsaAvx
    46, // VcmppsVpsHpsWpsIb -> X86Feature::IsaAvx
    46, // VcmppdVpdHpdWpdIb -> X86Feature::IsaAvx
    46, // VcmpssVssHpsWssIb -> X86Feature::IsaAvx
    46, // VcmpsdVsdHpdWsdIb -> X86Feature::IsaAvx
    46, // V128VpsrlwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsrlwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsrldVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsrldVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsrlqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsrlqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsrawVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsrawVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsradVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsradVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsllwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsllwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpslldVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpslldVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsllqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsllqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsrlwUdqIb -> X86Feature::IsaAvx
    47, // V256VpsrlwUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsrawUdqIb -> X86Feature::IsaAvx
    47, // V256VpsrawUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsllwUdqIb -> X86Feature::IsaAvx
    47, // V256VpsllwUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsrldUdqIb -> X86Feature::IsaAvx
    47, // V256VpsrldUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsradUdqIb -> X86Feature::IsaAvx
    47, // V256VpsradUdqIb -> X86Feature::IsaAvx2
    46, // V128VpslldUdqIb -> X86Feature::IsaAvx
    47, // V256VpslldUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsrlqUdqIb -> X86Feature::IsaAvx
    47, // V256VpsrlqUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsllqUdqIb -> X86Feature::IsaAvx
    47, // V256VpsllqUdqIb -> X86Feature::IsaAvx2
    46, // V128VpsrldqUdqIb -> X86Feature::IsaAvx
    47, // V256VpsrldqUdqIb -> X86Feature::IsaAvx2
    46, // V128VpslldqUdqIb -> X86Feature::IsaAvx
    47, // V256VpslldqUdqIb -> X86Feature::IsaAvx2
    46, // V128VpmovmskbGdUdq -> X86Feature::IsaAvx
    47, // V256VpmovmskbGdUdq -> X86Feature::IsaAvx2
    46, // VmovmskpsGdUps -> X86Feature::IsaAvx
    46, // VmovmskpdGdUpd -> X86Feature::IsaAvx
    46, // VunpcklpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VunpckhpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VunpcklpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VunpckhpsVpsHpsWps -> X86Feature::IsaAvx
    46, // V128VpunpckhdqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpckhdqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpckldqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpckldqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpcklbwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpcklbwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpcklwdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpcklwdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpckhbwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpckhbwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpckhwdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpckhwdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpcklqdqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpcklqdqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpunpckhqdqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpunpckhqdqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpeqbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpeqbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpeqwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpeqwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpeqdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpeqdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpeqqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpeqqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpgtbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpgtbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpgtwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpgtwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpgtdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpgtdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpcmpgtqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpcmpgtqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubsbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubsbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpaddsbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpaddsbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpaddswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpaddswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubusbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubusbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubuswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubuswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpaddusbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpaddusbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpadduswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpadduswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpavgbVdqWdq -> X86Feature::IsaAvx
    47, // V256VpavgbVdqWdq -> X86Feature::IsaAvx2
    46, // V128VpavgwVdqWdq -> X86Feature::IsaAvx
    47, // V256VpavgwVdqWdq -> X86Feature::IsaAvx2
    46, // V128VpandnVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpandnVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpandVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpandVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VporVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VporVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpxorVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpxorVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmulhrswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmulhrswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmuldqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmuldqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmuludqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmuludqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmulldVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmulldVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmullwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmullwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmulhwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmulhwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmulhuwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmulhuwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsadbwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsadbwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VmaskmovdquVdqUdq -> X86Feature::IsaAvx
    46, // V128VpsubbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsubqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsubqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpaddbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpaddbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpaddwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpaddwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpadddVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpadddVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpaddqVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpaddqVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpshufbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpshufbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphaddwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VphaddwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphadddVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VphadddVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphsubwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VphsubwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphsubdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VphsubdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphaddswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VphaddswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphsubswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VphsubswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaddwdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaddwdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaddubswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaddubswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsignbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsignbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsignwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsignwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpsigndVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpsigndVdqHdqWdq -> X86Feature::IsaAvx2
    46, // VtestpsVpsWps -> X86Feature::IsaAvx
    46, // VtestpdVpdWpd -> X86Feature::IsaAvx
    46, // VptestVdqWdq -> X86Feature::IsaAvx
    46, // VbroadcastssVpsMss -> X86Feature::IsaAvx
    46, // V256VbroadcastsdVpdMsd -> X86Feature::IsaAvx
    46, // V256Vbroadcastf128VdqMdq -> X86Feature::IsaAvx
    46, // V128VpabsbVdqWdq -> X86Feature::IsaAvx
    47, // V256VpabsbVdqWdq -> X86Feature::IsaAvx2
    46, // V128VpabswVdqWdq -> X86Feature::IsaAvx
    47, // V256VpabswVdqWdq -> X86Feature::IsaAvx2
    46, // V128VpabsdVdqWdq -> X86Feature::IsaAvx
    47, // V256VpabsdVdqWdq -> X86Feature::IsaAvx2
    46, // V128VpacksswbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpacksswbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpackuswbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpackuswbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpackusdwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpackusdwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpackssdwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpackssdwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // VmaskmovpsVpsHpsMps -> X86Feature::IsaAvx
    46, // VmaskmovpdVpdHpdMpd -> X86Feature::IsaAvx
    46, // VmaskmovpsMpsHpsVps -> X86Feature::IsaAvx
    46, // VmaskmovpdMpdHpdVpd -> X86Feature::IsaAvx
    46, // V128VpmovsxbwVdqWq -> X86Feature::IsaAvx
    46, // V128VpmovsxbdVdqWd -> X86Feature::IsaAvx
    46, // V128VpmovsxbqVdqWw -> X86Feature::IsaAvx
    46, // V128VpmovsxwdVdqWq -> X86Feature::IsaAvx
    46, // V128VpmovsxwqVdqWd -> X86Feature::IsaAvx
    46, // V128VpmovsxdqVdqWq -> X86Feature::IsaAvx
    46, // V128VpmovzxbwVdqWq -> X86Feature::IsaAvx
    46, // V128VpmovzxbdVdqWd -> X86Feature::IsaAvx
    46, // V128VpmovzxbqVdqWw -> X86Feature::IsaAvx
    46, // V128VpmovzxwdVdqWq -> X86Feature::IsaAvx
    46, // V128VpmovzxwqVdqWd -> X86Feature::IsaAvx
    46, // V128VpmovzxdqVdqWq -> X86Feature::IsaAvx
    46, // V128VpminsbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpminsbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpminswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpminswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpminsdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpminsdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpminubVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpminubVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpminuwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpminuwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpminudVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpminudVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaxsbVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaxsbVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaxswVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaxswVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaxsdVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaxsdVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaxubVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaxubVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaxuwVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaxuwVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VpmaxudVdqHdqWdq -> X86Feature::IsaAvx
    47, // V256VpmaxudVdqHdqWdq -> X86Feature::IsaAvx2
    46, // V128VphminposuwVdqWdq -> X86Feature::IsaAvx
    46, // VpermilpsVpsHpsWps -> X86Feature::IsaAvx
    46, // VpermilpdVpdHpdWpd -> X86Feature::IsaAvx
    46, // VpermilpsVpsWpsIb -> X86Feature::IsaAvx
    46, // VpermilpdVpdWpdIb -> X86Feature::IsaAvx
    46, // VblendpsVpsHpsWpsIb -> X86Feature::IsaAvx
    46, // VblendpdVpdHpdWpdIb -> X86Feature::IsaAvx
    46, // V128VpblendwVdqHdqWdqIb -> X86Feature::IsaAvx
    47, // V256VpblendwVdqHdqWdqIb -> X86Feature::IsaAvx2
    46, // V128VpalignrVdqHdqWdqIb -> X86Feature::IsaAvx
    47, // V256VpalignrVdqHdqWdqIb -> X86Feature::IsaAvx2
    46, // V128VinsertpsVpsWssIb -> X86Feature::IsaAvx
    46, // V128VextractpsEdVpsIb -> X86Feature::IsaAvx
    46, // V256Vperm2f128VdqHdqWdqIb -> X86Feature::IsaAvx
    46, // V256Vinsertf128VdqHdqWdqIb -> X86Feature::IsaAvx
    46, // V256Vextractf128WdqVdqIb -> X86Feature::IsaAvx
    46, // VblendvpsVpsHpsWpsIb -> X86Feature::IsaAvx
    46, // VblendvpdVpdHpdWpdIb -> X86Feature::IsaAvx
    46, // V128VpblendvbVdqHdqWdqIb -> X86Feature::IsaAvx
    47, // V256VpblendvbVdqHdqWdqIb -> X86Feature::IsaAvx2
    46, // V128VmpsadbwVdqHdqWdqIb -> X86Feature::IsaAvx
    47, // V256VmpsadbwVdqHdqWdqIb -> X86Feature::IsaAvx2
    46, // V128VpcmpestrmVdqWdqIb -> X86Feature::IsaAvx
    46, // V128VpcmpestriVdqWdqIb -> X86Feature::IsaAvx
    46, // V128VpcmpistrmVdqWdqIb -> X86Feature::IsaAvx
    46, // V128VpcmpistriVdqWdqIb -> X86Feature::IsaAvx
    46, // V128VaesimcVdqWdq -> X86Feature::IsaAvx
    46, // V128VaeskeygenassistVdqWdqIb -> X86Feature::IsaAvx
    46, // V128VaesencVdqHdqWdq -> X86Feature::IsaAvx
    46, // V128VaesenclastVdqHdqWdq -> X86Feature::IsaAvx
    46, // V128VaesdecVdqHdqWdq -> X86Feature::IsaAvx
    46, // V128VaesdeclastVdqHdqWdq -> X86Feature::IsaAvx
    46, // V128VpclmulqdqVdqHdqWdqIb -> X86Feature::IsaAvx
    43, // V256VaesencVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // V256VaesenclastVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // V256VaesdecVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // V256VaesdeclastVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // V256VpclmulqdqVdqHdqWdqIb -> X86Feature::IsaVaesVpclmulqdq
    69, // Vgf2p8affineqbVdqHdqWdqIb -> X86Feature::IsaGfni
    69, // Vgf2p8affineinvqbVdqHdqWdqIb -> X86Feature::IsaGfni
    69, // Vgf2p8mulbVdqHdqWdq -> X86Feature::IsaGfni
    70, // Vsm3msg1VdqHdqWdq -> X86Feature::IsaSm3
    70, // Vsm3msg2VdqHdqWdq -> X86Feature::IsaSm3
    70, // Vsm3rnds2VdqHdqWdqIb -> X86Feature::IsaSm3
    71, // Vsm4key4VdqHdqWdq -> X86Feature::IsaSm4
    71, // Vsm4rnds4VdqHdqWdq -> X86Feature::IsaSm4
    68, // Vsha512msg1VdqWdq -> X86Feature::IsaSha512
    68, // Vsha512msg2VdqWdq -> X86Feature::IsaSha512
    68, // Vsha512rnds2VdqHdqWdq -> X86Feature::IsaSha512
    46, // V128VmovdVdqEd -> X86Feature::IsaAvx
    46, // V128VmovqVdqEq -> X86Feature::IsaAvx
    46, // V128VmovdEdVd -> X86Feature::IsaAvx
    46, // V128VmovqEqVq -> X86Feature::IsaAvx
    46, // V128VpinsrbVdqEbIb -> X86Feature::IsaAvx
    46, // V128VpinsrwVdqEwIb -> X86Feature::IsaAvx
    46, // V128VpextrwGdUdqIb -> X86Feature::IsaAvx
    46, // V128VpextrbEdVdqIbR -> X86Feature::IsaAvx
    46, // V128VpextrbMbVdqIbM -> X86Feature::IsaAvx
    46, // V128VpextrwEdVdqIbR -> X86Feature::IsaAvx
    46, // V128VpextrwMwVdqIbM -> X86Feature::IsaAvx
    46, // V128VpinsrdVdqEdIb -> X86Feature::IsaAvx
    46, // V128VpinsrqVdqEqIb -> X86Feature::IsaAvx
    46, // V128VpextrdEdVdqIb -> X86Feature::IsaAvx
    46, // V128VpextrqEqVdqIb -> X86Feature::IsaAvx
    46, // Vcvtps2pdVpdWps -> X86Feature::IsaAvx
    46, // Vcvttpd2dqVdqWpd -> X86Feature::IsaAvx
    46, // Vcvtpd2dqVdqWpd -> X86Feature::IsaAvx
    46, // Vcvtdq2pdVpdWdq -> X86Feature::IsaAvx
    46, // Vcvtpd2psVpsWpd -> X86Feature::IsaAvx
    46, // Vcvtsd2ssVssWsd -> X86Feature::IsaAvx
    46, // Vcvtss2sdVsdWss -> X86Feature::IsaAvx
    46, // Vcvtdq2psVpsWdq -> X86Feature::IsaAvx
    46, // Vcvtps2dqVdqWps -> X86Feature::IsaAvx
    46, // Vcvttps2dqVdqWps -> X86Feature::IsaAvx
    46, // Vcvtss2siGdWss -> X86Feature::IsaAvx
    46, // Vcvtss2siGqWss -> X86Feature::IsaAvx
    46, // Vcvtsd2siGdWsd -> X86Feature::IsaAvx
    46, // Vcvtsd2siGqWsd -> X86Feature::IsaAvx
    46, // Vcvttss2siGdWss -> X86Feature::IsaAvx
    46, // Vcvttss2siGqWss -> X86Feature::IsaAvx
    46, // Vcvttsd2siGdWsd -> X86Feature::IsaAvx
    46, // Vcvttsd2siGqWsd -> X86Feature::IsaAvx
    46, // Vcvtsi2ssVssEd -> X86Feature::IsaAvx
    46, // Vcvtsi2ssVssEq -> X86Feature::IsaAvx
    46, // Vcvtsi2sdVsdEd -> X86Feature::IsaAvx
    46, // Vcvtsi2sdVsdEq -> X86Feature::IsaAvx
    46, // VmovqWqVq -> X86Feature::IsaAvx
    46, // VmovqVqWq -> X86Feature::IsaAvx
    48, // Vcvtph2psVpsWps -> X86Feature::IsaAvxF16c
    48, // Vcvtps2phWpsVpsIb -> X86Feature::IsaAvxF16c
    47, // V256VpmovsxbwVdqWdq -> X86Feature::IsaAvx2
    47, // V256VpmovsxbdVdqWq -> X86Feature::IsaAvx2
    47, // V256VpmovsxbqVdqWd -> X86Feature::IsaAvx2
    47, // V256VpmovsxwdVdqWdq -> X86Feature::IsaAvx2
    47, // V256VpmovsxwqVdqWq -> X86Feature::IsaAvx2
    47, // V256VpmovsxdqVdqWdq -> X86Feature::IsaAvx2
    47, // V256VpmovzxbwVdqWdq -> X86Feature::IsaAvx2
    47, // V256VpmovzxbdVdqWq -> X86Feature::IsaAvx2
    47, // V256VpmovzxbqVdqWd -> X86Feature::IsaAvx2
    47, // V256VpmovzxwdVdqWdq -> X86Feature::IsaAvx2
    47, // V256VpmovzxwqVdqWq -> X86Feature::IsaAvx2
    47, // V256VpmovzxdqVdqWdq -> X86Feature::IsaAvx2
    47, // V256Vperm2i128VdqHdqWdqIb -> X86Feature::IsaAvx2
    47, // V256Vinserti128VdqHdqWdqIb -> X86Feature::IsaAvx2
    47, // V256Vextracti128WdqVdqIb -> X86Feature::IsaAvx2
    47, // V256Vbroadcasti128VdqMdq -> X86Feature::IsaAvx2
    47, // VpbroadcastbVdqWb -> X86Feature::IsaAvx2
    47, // VpbroadcastwVdqWw -> X86Feature::IsaAvx2
    47, // VpbroadcastdVdqWd -> X86Feature::IsaAvx2
    47, // VpbroadcastqVdqWq -> X86Feature::IsaAvx2
    47, // VbroadcastssVpsWss -> X86Feature::IsaAvx2
    47, // V256VbroadcastsdVpdWsd -> X86Feature::IsaAvx2
    47, // VpblenddVdqHdqWdqIb -> X86Feature::IsaAvx2
    47, // VmaskmovdVdqHdqMdq -> X86Feature::IsaAvx2
    47, // VmaskmovqVdqHdqMdq -> X86Feature::IsaAvx2
    47, // VmaskmovdMdqHdqVdq -> X86Feature::IsaAvx2
    47, // VmaskmovqMdqHdqVdq -> X86Feature::IsaAvx2
    47, // VgatherdpsVpsHps -> X86Feature::IsaAvx2
    47, // VgatherdpdVpdHpd -> X86Feature::IsaAvx2
    47, // VgatherqpsVpsHps -> X86Feature::IsaAvx2
    47, // VgatherqpdVpdHpd -> X86Feature::IsaAvx2
    47, // VgatherddVdqHdq -> X86Feature::IsaAvx2
    47, // VgatherdqVdqHdq -> X86Feature::IsaAvx2
    47, // VgatherqdVdqHdq -> X86Feature::IsaAvx2
    47, // VgatherqqVdqHdq -> X86Feature::IsaAvx2
    47, // VpsrlvdVdqHdqWdq -> X86Feature::IsaAvx2
    47, // VpsrlvqVdqHdqWdq -> X86Feature::IsaAvx2
    47, // VpsllvdVdqHdqWdq -> X86Feature::IsaAvx2
    47, // VpsllvqVdqHdqWdq -> X86Feature::IsaAvx2
    47, // V256VpermqVdqWdqIb -> X86Feature::IsaAvx2
    47, // V256VpermdVdqHdqWdq -> X86Feature::IsaAvx2
    47, // V256VpermpsVpsHpsWps -> X86Feature::IsaAvx2
    47, // V256VpermpdVpdWpdIb -> X86Feature::IsaAvx2
    47, // VpsravdVdqHdqWdq -> X86Feature::IsaAvx2
    49, // Vfmadd132psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmadd132pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmadd213psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmadd213pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmadd231psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmadd231pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmadd132ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfmadd132sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfmadd213ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfmadd213sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfmadd231ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfmadd231sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfmaddsub132psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmaddsub132pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmaddsub213psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmaddsub213pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmaddsub231psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmaddsub231pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsubadd132psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmsubadd132pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsubadd213psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmsubadd213pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsubadd231psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmsubadd231pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsub132psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmsub132pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsub213psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmsub213pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsub231psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfmsub231pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfmsub132ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfmsub132sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfmsub213ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfmsub213sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfmsub231ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfmsub231sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfnmadd132psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfnmadd132pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfnmadd213psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfnmadd213pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfnmadd231psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfnmadd231pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfnmadd132ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfnmadd132sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfnmadd213ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfnmadd213sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfnmadd231ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfnmadd231sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfnmsub132psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfnmsub132pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfnmsub213psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfnmsub213pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfnmsub231psVpsHpsWps -> X86Feature::IsaAvxFma
    49, // Vfnmsub231pdVpdHpdWpd -> X86Feature::IsaAvxFma
    49, // Vfnmsub132ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfnmsub132sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfnmsub213ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfnmsub213sdVpdHsdWsd -> X86Feature::IsaAvxFma
    49, // Vfnmsub231ssVpsHssWss -> X86Feature::IsaAvxFma
    49, // Vfnmsub231sdVpdHsdWsd -> X86Feature::IsaAvxFma
    73, // VpdpbusdVdqHdqWdq -> X86Feature::IsaAvxVnni
    73, // VpdpbusdsVdqHdqWdq -> X86Feature::IsaAvxVnni
    73, // VpdpwssdVdqHdqWdq -> X86Feature::IsaAvxVnni
    73, // VpdpwssdsVdqHdqWdq -> X86Feature::IsaAvxVnni
    72, // Vpmadd52luqVdqHdqWdq -> X86Feature::IsaAvxIfma
    72, // Vpmadd52huqVdqHdqWdq -> X86Feature::IsaAvxIfma
    74, // VpdpbssdVdqHdqWdq -> X86Feature::IsaAvxVnniInt8
    74, // VpdpbssdsVdqHdqWdq -> X86Feature::IsaAvxVnniInt8
    74, // VpdpbsudVdqHdqWdq -> X86Feature::IsaAvxVnniInt8
    74, // VpdpbsudsVdqHdqWdq -> X86Feature::IsaAvxVnniInt8
    74, // VpdpbuudVdqHdqWdq -> X86Feature::IsaAvxVnniInt8
    74, // VpdpbuudsVdqHdqWdq -> X86Feature::IsaAvxVnniInt8
    75, // VpdpwsudVdqHdqWdq -> X86Feature::IsaAvxVnniInt16
    75, // VpdpwsudsVdqHdqWdq -> X86Feature::IsaAvxVnniInt16
    75, // VpdpwusdVdqHdqWdq -> X86Feature::IsaAvxVnniInt16
    75, // VpdpwusdsVdqHdqWdq -> X86Feature::IsaAvxVnniInt16
    75, // VpdpwuudVdqHdqWdq -> X86Feature::IsaAvxVnniInt16
    75, // VpdpwuudsVdqHdqWdq -> X86Feature::IsaAvxVnniInt16
    76, // Vbcstnebf162psVpsWw -> X86Feature::IsaAvxNeConvert
    76, // Vbcstnesh2psVpsWsh -> X86Feature::IsaAvxNeConvert
    76, // Vcvtneeph2psVpsWph -> X86Feature::IsaAvxNeConvert
    76, // Vcvtneoph2psVpsWph -> X86Feature::IsaAvxNeConvert
    76, // Vcvtneebf162psVpsWph -> X86Feature::IsaAvxNeConvert
    76, // Vcvtneobf162psVpsWph -> X86Feature::IsaAvxNeConvert
    76, // Vcvtneps2bf16VphWps -> X86Feature::IsaAvxNeConvert
    54, // AndnGdBdEd -> X86Feature::IsaBmi1
    54, // AndnGqBqEq -> X86Feature::IsaBmi1
    54, // BlsiBdEd -> X86Feature::IsaBmi1
    54, // BlsiBqEq -> X86Feature::IsaBmi1
    54, // BlsmskBdEd -> X86Feature::IsaBmi1
    54, // BlsmskBqEq -> X86Feature::IsaBmi1
    54, // BlsrBdEd -> X86Feature::IsaBmi1
    54, // BlsrBqEq -> X86Feature::IsaBmi1
    54, // BextrGdEdBd -> X86Feature::IsaBmi1
    54, // BextrGqEqBq -> X86Feature::IsaBmi1
    55, // MulxGdBdEd -> X86Feature::IsaBmi2
    55, // MulxGqBqEq -> X86Feature::IsaBmi2
    55, // RorxGdEdIb -> X86Feature::IsaBmi2
    55, // RorxGqEqIb -> X86Feature::IsaBmi2
    55, // ShlxGdEdBd -> X86Feature::IsaBmi2
    55, // ShlxGqEqBq -> X86Feature::IsaBmi2
    55, // ShrxGdEdBd -> X86Feature::IsaBmi2
    55, // ShrxGqEqBq -> X86Feature::IsaBmi2
    55, // SarxGdEdBd -> X86Feature::IsaBmi2
    55, // SarxGqEqBq -> X86Feature::IsaBmi2
    55, // BzhiGdBdEd -> X86Feature::IsaBmi2
    55, // BzhiGqBqEq -> X86Feature::IsaBmi2
    55, // PextGdBdEd -> X86Feature::IsaBmi2
    55, // PextGqBqEq -> X86Feature::IsaBmi2
    55, // PdepGdBdEd -> X86Feature::IsaBmi2
    55, // PdepGqBqEq -> X86Feature::IsaBmi2
    122, // CmpbexaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpbexaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpbxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpbxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmplexaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmplexaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmplxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmplxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnbexaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnbexaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnbxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnbxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnlexaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnlexaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnlxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnlxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnoxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnoxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnpxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnpxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnsxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnsxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpnzxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpnzxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpoxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpoxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmppxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmppxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpsxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpsxaddEqGqBq -> X86Feature::IsaCmpccxadd
    122, // CmpzxaddEdGdBd -> X86Feature::IsaCmpccxadd
    122, // CmpzxaddEqGqBq -> X86Feature::IsaCmpccxadd
    56, // VfmaddsubpsVpsHpsVibWps -> X86Feature::IsaFma4
    56, // VfmaddsubpsVpsHpsWpsVib -> X86Feature::IsaFma4
    56, // VfmaddsubpdVpdHpdVibWpd -> X86Feature::IsaFma4
    56, // VfmaddsubpdVpdHpdWpdVib -> X86Feature::IsaFma4
    56, // VfmsubaddpsVpsHpsVibWps -> X86Feature::IsaFma4
    56, // VfmsubaddpsVpsHpsWpsVib -> X86Feature::IsaFma4
    56, // VfmsubaddpdVpdHpdVibWpd -> X86Feature::IsaFma4
    56, // VfmsubaddpdVpdHpdWpdVib -> X86Feature::IsaFma4
    56, // VfmaddpsVpsHpsVibWps -> X86Feature::IsaFma4
    56, // VfmaddpsVpsHpsWpsVib -> X86Feature::IsaFma4
    56, // VfmaddpdVpdHpdVibWpd -> X86Feature::IsaFma4
    56, // VfmaddpdVpdHpdWpdVib -> X86Feature::IsaFma4
    56, // VfmaddssVssHssVibWss -> X86Feature::IsaFma4
    56, // VfmaddssVssHssWssVib -> X86Feature::IsaFma4
    56, // VfmaddsdVsdHsdVibWsd -> X86Feature::IsaFma4
    56, // VfmaddsdVsdHsdWsdVib -> X86Feature::IsaFma4
    56, // VfmsubpsVpsHpsVibWps -> X86Feature::IsaFma4
    56, // VfmsubpsVpsHpsWpsVib -> X86Feature::IsaFma4
    56, // VfmsubpdVpdHpdVibWpd -> X86Feature::IsaFma4
    56, // VfmsubpdVpdHpdWpdVib -> X86Feature::IsaFma4
    56, // VfmsubssVssHssVibWss -> X86Feature::IsaFma4
    56, // VfmsubssVssHssWssVib -> X86Feature::IsaFma4
    56, // VfmsubsdVsdHsdVibWsd -> X86Feature::IsaFma4
    56, // VfmsubsdVsdHsdWsdVib -> X86Feature::IsaFma4
    56, // VfnmaddpsVpsHpsVibWps -> X86Feature::IsaFma4
    56, // VfnmaddpsVpsHpsWpsVib -> X86Feature::IsaFma4
    56, // VfnmaddpdVpdHpdVibWpd -> X86Feature::IsaFma4
    56, // VfnmaddpdVpdHpdWpdVib -> X86Feature::IsaFma4
    56, // VfnmaddssVssHssVibWss -> X86Feature::IsaFma4
    56, // VfnmaddssVssHssWssVib -> X86Feature::IsaFma4
    56, // VfnmaddsdVsdHsdVibWsd -> X86Feature::IsaFma4
    56, // VfnmaddsdVsdHsdWsdVib -> X86Feature::IsaFma4
    56, // VfnmsubpsVpsHpsVibWps -> X86Feature::IsaFma4
    56, // VfnmsubpsVpsHpsWpsVib -> X86Feature::IsaFma4
    56, // VfnmsubpdVpdHpdVibWpd -> X86Feature::IsaFma4
    56, // VfnmsubpdVpdHpdWpdVib -> X86Feature::IsaFma4
    56, // VfnmsubssVssHssVibWss -> X86Feature::IsaFma4
    56, // VfnmsubssVssHssWssVib -> X86Feature::IsaFma4
    56, // VfnmsubsdVsdHsdVibWsd -> X86Feature::IsaFma4
    56, // VfnmsubsdVsdHsdWsdVib -> X86Feature::IsaFma4
    57, // VpcmovVdqHdqVibWdq -> X86Feature::IsaXop
    57, // VpcmovVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VppermVdqHdqVibWdq -> X86Feature::IsaXop
    57, // VppermVdqHdqWdqVib -> X86Feature::IsaXop
    57, // Vpermil2psVdqHdqVibWdq -> X86Feature::IsaXop
    57, // Vpermil2psVdqHdqWdqVib -> X86Feature::IsaXop
    57, // Vpermil2pdVdqHdqVibWdq -> X86Feature::IsaXop
    57, // Vpermil2pdVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpshabVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshabVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshawVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshawVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshadVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshadVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshaqVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshaqVdqWdqHdq -> X86Feature::IsaXop
    57, // VprotbVdqHdqWdq -> X86Feature::IsaXop
    57, // VprotbVdqWdqHdq -> X86Feature::IsaXop
    57, // VprotwVdqHdqWdq -> X86Feature::IsaXop
    57, // VprotwVdqWdqHdq -> X86Feature::IsaXop
    57, // VprotdVdqHdqWdq -> X86Feature::IsaXop
    57, // VprotdVdqWdqHdq -> X86Feature::IsaXop
    57, // VprotqVdqHdqWdq -> X86Feature::IsaXop
    57, // VprotqVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshlbVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshlbVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshlwVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshlwVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshldVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshldVdqWdqHdq -> X86Feature::IsaXop
    57, // VpshlqVdqHdqWdq -> X86Feature::IsaXop
    57, // VpshlqVdqWdqHdq -> X86Feature::IsaXop
    57, // VpmacsswwVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacsswdVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacssdqlVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacssddVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacssdqhVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacswwVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacswdVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacsdqlVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacsddVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmacsdqhVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmadcsswdVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VpmadcswdVdqHdqWdqVib -> X86Feature::IsaXop
    57, // VprotbVdqWdqIb -> X86Feature::IsaXop
    57, // VprotwVdqWdqIb -> X86Feature::IsaXop
    57, // VprotdVdqWdqIb -> X86Feature::IsaXop
    57, // VprotqVdqWdqIb -> X86Feature::IsaXop
    57, // VpcombVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomwVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomdVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomqVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomubVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomuwVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomudVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VpcomuqVdqHdqWdqIb -> X86Feature::IsaXop
    57, // VfrczpsVpsWps -> X86Feature::IsaXop
    57, // VfrczpdVpdWpd -> X86Feature::IsaXop
    57, // VfrczssVssWss -> X86Feature::IsaXop
    57, // VfrczsdVsdWsd -> X86Feature::IsaXop
    57, // VphaddbwVdqWdq -> X86Feature::IsaXop
    57, // VphaddbdVdqWdq -> X86Feature::IsaXop
    57, // VphaddbqVdqWdq -> X86Feature::IsaXop
    57, // VphaddwdVdqWdq -> X86Feature::IsaXop
    57, // VphaddwqVdqWdq -> X86Feature::IsaXop
    57, // VphadddqVdqWdq -> X86Feature::IsaXop
    57, // VphaddubwVdqWdq -> X86Feature::IsaXop
    57, // VphaddubdVdqWdq -> X86Feature::IsaXop
    57, // VphaddubqVdqWdq -> X86Feature::IsaXop
    57, // VphadduwdVdqWdq -> X86Feature::IsaXop
    57, // VphadduwqVdqWdq -> X86Feature::IsaXop
    57, // VphaddudqVdqWdq -> X86Feature::IsaXop
    57, // VphsubbwVdqWdq -> X86Feature::IsaXop
    57, // VphsubwdVdqWdq -> X86Feature::IsaXop
    57, // VphsubdqVdqWdq -> X86Feature::IsaXop
    58, // BextrGdEdId -> X86Feature::IsaTbm
    58, // BextrGqEqId -> X86Feature::IsaTbm
    58, // BlcfillBdEd -> X86Feature::IsaTbm
    58, // BlcfillBqEq -> X86Feature::IsaTbm
    58, // BlciBdEd -> X86Feature::IsaTbm
    58, // BlciBqEq -> X86Feature::IsaTbm
    58, // BlcicBdEd -> X86Feature::IsaTbm
    58, // BlcicBqEq -> X86Feature::IsaTbm
    58, // BlcmskBdEd -> X86Feature::IsaTbm
    58, // BlcmskBqEq -> X86Feature::IsaTbm
    58, // BlcsBdEd -> X86Feature::IsaTbm
    58, // BlcsBqEq -> X86Feature::IsaTbm
    58, // BlsfillBdEd -> X86Feature::IsaTbm
    58, // BlsfillBqEq -> X86Feature::IsaTbm
    58, // BlsicBdEd -> X86Feature::IsaTbm
    58, // BlsicBqEq -> X86Feature::IsaTbm
    58, // T1mskcBdEd -> X86Feature::IsaTbm
    58, // T1mskcBqEq -> X86Feature::IsaTbm
    58, // TzmskBdEd -> X86Feature::IsaTbm
    58, // TzmskBqEq -> X86Feature::IsaTbm
    54, // TzcntGwEw -> X86Feature::IsaBmi1
    54, // TzcntGdEd -> X86Feature::IsaBmi1
    54, // TzcntGqEq -> X86Feature::IsaBmi1
    53, // LzcntGwEw -> X86Feature::IsaLzcnt
    53, // LzcntGdEd -> X86Feature::IsaLzcnt
    53, // LzcntGqEq -> X86Feature::IsaLzcnt
    50, // MovntssMssVss -> X86Feature::IsaSse4a
    50, // MovntsdMsdVsd -> X86Feature::IsaSse4a
    50, // ExtrqUdqIbIb -> X86Feature::IsaSse4a
    50, // ExtrqVdqUq -> X86Feature::IsaSse4a
    50, // InsertqVdqUqIbIb -> X86Feature::IsaSse4a
    50, // InsertqVdqUdq -> X86Feature::IsaSse4a
    64, // AdcxGdEd -> X86Feature::IsaAdx
    64, // AdoxGdEd -> X86Feature::IsaAdx
    64, // AdcxGqEq -> X86Feature::IsaAdx
    64, // AdoxGqEq -> X86Feature::IsaAdx
    65, // Stac -> X86Feature::IsaSmap
    65, // Clac -> X86Feature::IsaSmap
    62, // RdrandEw -> X86Feature::IsaRdrand
    62, // RdrandEd -> X86Feature::IsaRdrand
    62, // RdrandEq -> X86Feature::IsaRdrand
    63, // RdseedEw -> X86Feature::IsaRdseed
    63, // RdseedEd -> X86Feature::IsaRdseed
    63, // RdseedEq -> X86Feature::IsaRdseed
    128, // MovdiriMdGd -> X86Feature::IsaMovdiri
    128, // MovdiriMqGq -> X86Feature::IsaMovdiri
    129, // Movdir64bGdMdq -> X86Feature::IsaMovdir64b
    129, // Movdir64bGqMdq -> X86Feature::IsaMovdir64b
    131, // AaddEdGd -> X86Feature::IsaRaoInt
    131, // AandEdGd -> X86Feature::IsaRaoInt
    131, // AorEdGd -> X86Feature::IsaRaoInt
    131, // AxorEdGd -> X86Feature::IsaRaoInt
    131, // AaddEqGq -> X86Feature::IsaRaoInt
    131, // AandEqGq -> X86Feature::IsaRaoInt
    131, // AorEqGq -> X86Feature::IsaRaoInt
    131, // AxorEqGq -> X86Feature::IsaRaoInt
    90, // Ldtilecfg -> X86Feature::IsaAmx
    90, // Sttilecfg -> X86Feature::IsaAmx
    90, // TileloaddTnnnMdq -> X86Feature::IsaAmx
    90, // Tileloaddt1TnnnMdq -> X86Feature::IsaAmx
    97, // TileloaddrsTnnnMdq -> X86Feature::IsaAmxMovrs
    97, // Tileloaddrst1TnnnMdq -> X86Feature::IsaAmxMovrs
    90, // TilestoredMdqTnnn -> X86Feature::IsaAmx
    90, // Tilerelease -> X86Feature::IsaAmx
    90, // TilezeroTnnn -> X86Feature::IsaAmx
    91, // TdpbssdTnnnTrmTreg -> X86Feature::IsaAmxInt8
    91, // TdpbsudTnnnTrmTreg -> X86Feature::IsaAmxInt8
    91, // TdpbusdTnnnTrmTreg -> X86Feature::IsaAmxInt8
    91, // TdpbuudTnnnTrmTreg -> X86Feature::IsaAmxInt8
    92, // Tdpbf16psTnnnTrmTreg -> X86Feature::IsaAmxBf16
    93, // Tdpfp16psTnnnTrmTreg -> X86Feature::IsaAmxFp16
    96, // Tcmmrlfp16psTnnnTrmTreg -> X86Feature::IsaAmxComplex
    96, // Tcmmimfp16psTnnnTrmTreg -> X86Feature::IsaAmxComplex
    ISA_ALWAYS, // Tmmultf32psTnnnTrmTreg
    95, // Tdpbf8psTnnnTrmTreg -> X86Feature::IsaAmxFp8
    95, // Tdphf8psTnnnTrmTreg -> X86Feature::IsaAmxFp8
    95, // Tdpbhf8psTnnnTrmTreg -> X86Feature::IsaAmxFp8
    95, // Tdphbf8psTnnnTrmTreg -> X86Feature::IsaAmxFp8
    78, // KaddwKgwKhwKew -> X86Feature::IsaAvx512Dq
    79, // KaddqKgqKhqKeq -> X86Feature::IsaAvx512Bw
    78, // KaddbKgbKhbKeb -> X86Feature::IsaAvx512Dq
    79, // KadddKgdKhdKed -> X86Feature::IsaAvx512Bw
    77, // KandwKgwKhwKew -> X86Feature::IsaAvx512
    79, // KandqKgqKhqKeq -> X86Feature::IsaAvx512Bw
    78, // KandbKgbKhbKeb -> X86Feature::IsaAvx512Dq
    79, // KanddKgdKhdKed -> X86Feature::IsaAvx512Bw
    77, // KandnwKgwKhwKew -> X86Feature::IsaAvx512
    79, // KandnqKgqKhqKeq -> X86Feature::IsaAvx512Bw
    78, // KandnbKgbKhbKeb -> X86Feature::IsaAvx512Dq
    79, // KandndKgdKhdKed -> X86Feature::IsaAvx512Bw
    77, // KmovwKgwKew -> X86Feature::IsaAvx512
    79, // KmovqKgqKeq -> X86Feature::IsaAvx512Bw
    78, // KmovbKgbKeb -> X86Feature::IsaAvx512Dq
    79, // KmovdKgdKed -> X86Feature::IsaAvx512Bw
    77, // KmovwKewKgw -> X86Feature::IsaAvx512
    79, // KmovqKeqKgq -> X86Feature::IsaAvx512Bw
    78, // KmovbKebKgb -> X86Feature::IsaAvx512Dq
    79, // KmovdKedKgd -> X86Feature::IsaAvx512Bw
    78, // KmovbGdKeb -> X86Feature::IsaAvx512Dq
    77, // KmovwGdKew -> X86Feature::IsaAvx512
    79, // KmovdGdKed -> X86Feature::IsaAvx512Bw
    79, // KmovqGqKeq -> X86Feature::IsaAvx512Bw
    78, // KmovbKgbEb -> X86Feature::IsaAvx512Dq
    77, // KmovwKgwEw -> X86Feature::IsaAvx512
    79, // KmovdKgdEd -> X86Feature::IsaAvx512Bw
    79, // KmovqKgqEq -> X86Feature::IsaAvx512Bw
    77, // KunpckbwKgwKhbKeb -> X86Feature::IsaAvx512
    79, // KunpckwdKgdKhwKew -> X86Feature::IsaAvx512Bw
    79, // KunpckdqKgqKhdKed -> X86Feature::IsaAvx512Bw
    77, // KnotwKgwKew -> X86Feature::IsaAvx512
    79, // KnotqKgqKeq -> X86Feature::IsaAvx512Bw
    78, // KnotbKgbKeb -> X86Feature::IsaAvx512Dq
    79, // KnotdKgdKed -> X86Feature::IsaAvx512Bw
    77, // KorwKgwKhwKew -> X86Feature::IsaAvx512
    79, // KorqKgqKhqKeq -> X86Feature::IsaAvx512Bw
    78, // KorbKgbKhbKeb -> X86Feature::IsaAvx512Dq
    79, // KordKgdKhdKed -> X86Feature::IsaAvx512Bw
    77, // KortestwKgwKew -> X86Feature::IsaAvx512
    79, // KortestqKgqKeq -> X86Feature::IsaAvx512Bw
    78, // KortestbKgbKeb -> X86Feature::IsaAvx512Dq
    79, // KortestdKgdKed -> X86Feature::IsaAvx512Bw
    78, // KshiftlbKgbKebIb -> X86Feature::IsaAvx512Dq
    77, // KshiftlwKgwKewIb -> X86Feature::IsaAvx512
    79, // KshiftldKgdKedIb -> X86Feature::IsaAvx512Bw
    79, // KshiftlqKgqKeqIb -> X86Feature::IsaAvx512Bw
    78, // KshiftrbKgbKebIb -> X86Feature::IsaAvx512Dq
    77, // KshiftrwKgwKewIb -> X86Feature::IsaAvx512
    79, // KshiftrdKgdKedIb -> X86Feature::IsaAvx512Bw
    79, // KshiftrqKgqKeqIb -> X86Feature::IsaAvx512Bw
    77, // KxnorwKgwKhwKew -> X86Feature::IsaAvx512
    79, // KxnorqKgqKhqKeq -> X86Feature::IsaAvx512Bw
    78, // KxnorbKgbKhbKeb -> X86Feature::IsaAvx512Dq
    79, // KxnordKgdKhdKed -> X86Feature::IsaAvx512Bw
    77, // KxorwKgwKhwKew -> X86Feature::IsaAvx512
    79, // KxorqKgqKhqKeq -> X86Feature::IsaAvx512Bw
    78, // KxorbKgbKhbKeb -> X86Feature::IsaAvx512Dq
    79, // KxordKgdKhdKed -> X86Feature::IsaAvx512Bw
    78, // KtestwKgwKew -> X86Feature::IsaAvx512Dq
    79, // KtestqKgqKeq -> X86Feature::IsaAvx512Bw
    78, // KtestbKgbKeb -> X86Feature::IsaAvx512Dq
    79, // KtestdKgdKed -> X86Feature::IsaAvx512Bw
    121, // RdmsrEqId -> X86Feature::IsaMsrImm
    121, // WrmsrnsIdEq -> X86Feature::IsaMsrImm
    132, // MovrsGbEb -> X86Feature::IsaMovrs
    132, // MovrsGwEw -> X86Feature::IsaMovrs
    132, // MovrsGdEd -> X86Feature::IsaMovrs
    132, // MovrsGqEq -> X86Feature::IsaMovrs
    133, // Erets -> X86Feature::IsaFred
    133, // Eretu -> X86Feature::IsaFred
    133, // LkgsEw -> X86Feature::IsaFred
    77, // EvexVaddpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVaddpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVaddssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVaddsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVaddpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVaddpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVaddssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVaddsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVsubpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVsubpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVsubssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVsubsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVsubpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVsubpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVsubssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVsubsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmulpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVmulpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVmulssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVmulsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVmulpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmulpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmulssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVmulsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVdivpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVdivpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVdivssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVdivsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVdivpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVdivpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVdivssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVdivsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVminpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVminpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVminssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVminsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVminpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVminpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVminssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVminsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmaxpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVmaxpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVmaxssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVmaxsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVmaxpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmaxpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmaxssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVmaxsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVsqrtpsVpsWps -> X86Feature::IsaAvx512
    77, // EvexVsqrtpdVpdWpd -> X86Feature::IsaAvx512
    77, // EvexVsqrtssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVsqrtsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVsqrtpsVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVsqrtpdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVsqrtssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVsqrtsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVcmppsKgwHpsWpsIb -> X86Feature::IsaAvx512
    77, // EvexVcmppdKgbHpdWpdIb -> X86Feature::IsaAvx512
    77, // EvexVcmpssKgbHssWssIb -> X86Feature::IsaAvx512
    77, // EvexVcmpsdKgbHsdWsdIb -> X86Feature::IsaAvx512
    77, // EvexVrndscalepsVpsWpsIbKmask -> X86Feature::IsaAvx512
    77, // EvexVrndscalepdVpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVrndscalessVssHpsWssIbKmask -> X86Feature::IsaAvx512
    77, // EvexVrndscalesdVsdHpdWsdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVunpcklpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVunpcklpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVunpcklpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVunpcklpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVunpckhpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVunpckhpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVunpckhpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVunpckhpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVpunpckldqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpunpcklqdqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpunpckldqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpunpcklqdqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpunpckhdqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpunpckhqdqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpunpckhdqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpunpckhqdqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmuldqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmuludqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmuldqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmuludqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVucomissVssWss -> X86Feature::IsaAvx512
    77, // EvexVcomissVssWss -> X86Feature::IsaAvx512
    77, // EvexVucomisdVsdWsd -> X86Feature::IsaAvx512
    77, // EvexVcomisdVsdWsd -> X86Feature::IsaAvx512
    77, // EvexVcvtss2sdVsdWss -> X86Feature::IsaAvx512
    77, // EvexVcvtsd2ssVssWsd -> X86Feature::IsaAvx512
    77, // EvexVcvtps2pdVpdWps -> X86Feature::IsaAvx512
    77, // EvexVcvtpd2psVpsWpd -> X86Feature::IsaAvx512
    77, // EvexVcvtss2sdVsdWssKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtsd2ssVssWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtps2pdVpdWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtpd2psVpsWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtps2dqVdqWps -> X86Feature::IsaAvx512
    77, // EvexVcvtps2dqVdqWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcvttps2dqVdqWps -> X86Feature::IsaAvx512
    77, // EvexVcvttps2dqVdqWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtpd2dqVdqWpd -> X86Feature::IsaAvx512
    77, // EvexVcvtpd2dqVdqWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVcvttpd2dqVdqWpd -> X86Feature::IsaAvx512
    77, // EvexVcvttpd2dqVdqWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtph2psVpsWps -> X86Feature::IsaAvx512
    77, // EvexVcvtph2psVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtps2phWpsVpsIb -> X86Feature::IsaAvx512
    77, // EvexVcvtps2phWpsVpsIbKmask -> X86Feature::IsaAvx512
    88, // EvexVcvtneps2bf16VphWpsKmask -> X86Feature::IsaAvx512Bf16
    88, // EvexVcvtne2ps2bf16VphHpsWpsKmask -> X86Feature::IsaAvx512Bf16
    88, // EvexVdpbf16psVpsHdqWdqKmask -> X86Feature::IsaAvx512Bf16
    77, // EvexVmovapsVpsWps -> X86Feature::IsaAvx512
    77, // EvexVmovapsVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmovapsWpsVps -> X86Feature::IsaAvx512
    77, // EvexVmovapsWpsVpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmovapdVpdWpd -> X86Feature::IsaAvx512
    77, // EvexVmovapdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovapdWpdVpd -> X86Feature::IsaAvx512
    77, // EvexVmovapdWpdVpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovupsVpsWps -> X86Feature::IsaAvx512
    77, // EvexVmovupsVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmovupsWpsVps -> X86Feature::IsaAvx512
    77, // EvexVmovupsWpsVpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmovupdVpdWpd -> X86Feature::IsaAvx512
    77, // EvexVmovupdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovupdWpdVpd -> X86Feature::IsaAvx512
    77, // EvexVmovupdWpdVpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVmovssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVmovsdWsdHpdVsd -> X86Feature::IsaAvx512
    77, // EvexVmovssWssHpsVss -> X86Feature::IsaAvx512
    77, // EvexVmovsdVsdWsd -> X86Feature::IsaAvx512
    77, // EvexVmovssVssWss -> X86Feature::IsaAvx512
    77, // EvexVmovsdWsdVsd -> X86Feature::IsaAvx512
    77, // EvexVmovssWssVss -> X86Feature::IsaAvx512
    77, // EvexVmovsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVmovsdWsdHpdVsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovssWssHpsVssKmask -> X86Feature::IsaAvx512
    77, // EvexVmovsdVsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovssVssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVmovsdWsdVsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovssWssVssKmask -> X86Feature::IsaAvx512
    79, // EvexVpabsbVdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpabswVdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpabsdVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpabsqVdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpabsbVdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpabswVdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpabsdVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpabsqVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovntdqaVdqMdq -> X86Feature::IsaAvx512
    77, // EvexVmovntpsMpsVps -> X86Feature::IsaAvx512
    77, // EvexVmovntpdMpdVpd -> X86Feature::IsaAvx512
    77, // EvexVmovntdqMdqVdq -> X86Feature::IsaAvx512
    79, // EvexVpcmpeqbKgqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpcmpeqwKgdHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpcmpgtbKgqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpcmpgtwKgdHdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpcmpeqdKgwHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpcmpeqqKgbHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpcmpgtdKgwHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpcmpgtqKgbHdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpsrlwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrlwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrawVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrawVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsllwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsllwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrlwUdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrlwUdqIbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsllwUdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpsllwUdqIbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrawUdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrawUdqIbKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpsrldVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsrlqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsrldVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsrlqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpslldVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsllqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpslldVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsllqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsrldUdqIb -> X86Feature::IsaAvx512
    77, // EvexVpsrldUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpsrlqUdqIb -> X86Feature::IsaAvx512
    77, // EvexVpsrlqUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpslldUdqIb -> X86Feature::IsaAvx512
    77, // EvexVpslldUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpsllqUdqIb -> X86Feature::IsaAvx512
    77, // EvexVpsllqUdqIbKmask -> X86Feature::IsaAvx512
    79, // EvexVpshufbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpshufbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpermqVdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpermpdVpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVshufpsVpsHpsWpsIb -> X86Feature::IsaAvx512
    77, // EvexVshufpdVpdHpdWpdIb -> X86Feature::IsaAvx512
    77, // EvexVshufpsVpsHpsWpsIbKmask -> X86Feature::IsaAvx512
    77, // EvexVshufpdVpdHpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpermilpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVpermilpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVpermilpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVpermilpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVpermilpsVpsWpsIb -> X86Feature::IsaAvx512
    77, // EvexVpermilpdVpdWpdIb -> X86Feature::IsaAvx512
    77, // EvexVpermilpsVpsWpsIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpermilpdVpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpshufdVdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpshufdVdqWdqIbKmask -> X86Feature::IsaAvx512
    79, // EvexVpshuflwVdqWdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpshuflwVdqWdqIbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpshufhwVdqWdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpshufhwVdqWdqIbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastbVdqEb -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastbVdqEbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastwVdqEw -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastwVdqEwKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpbroadcastdVdqEd -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastdVdqEdKmask -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastqVdqEq -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastqVdqEqKmask -> X86Feature::IsaAvx512
    79, // EvexVpbroadcastbVdqWb -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastbVdqWbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastwVdqWw -> X86Feature::IsaAvx512Bw
    79, // EvexVpbroadcastwVdqWwKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpbroadcastdVdqWd -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastdVdqWdKmask -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastqVdqWq -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastqVdqWqKmask -> X86Feature::IsaAvx512
    77, // EvexVbroadcastssVpsWss -> X86Feature::IsaAvx512
    77, // EvexVbroadcastssVpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVbroadcastsdVpdWsd -> X86Feature::IsaAvx512
    77, // EvexVbroadcastsdVpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovqWqVq -> X86Feature::IsaAvx512
    77, // EvexVmovqVqWq -> X86Feature::IsaAvx512
    77, // EvexVinsertpsVpsWssIb -> X86Feature::IsaAvx512
    77, // EvexVextractpsEdVpsIb -> X86Feature::IsaAvx512
    77, // EvexVmovlpsVpsHpsMq -> X86Feature::IsaAvx512
    77, // EvexVmovhlpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVmovhpsVpsHpsMq -> X86Feature::IsaAvx512
    77, // EvexVmovlhpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVmovlpsMqVps -> X86Feature::IsaAvx512
    77, // EvexVmovhpsMqVps -> X86Feature::IsaAvx512
    77, // EvexVmovlpdMqVsd -> X86Feature::IsaAvx512
    77, // EvexVmovhpdMqVsd -> X86Feature::IsaAvx512
    77, // EvexVmovlpdVpdHpdMq -> X86Feature::IsaAvx512
    77, // EvexVmovhpdVpdHpdMq -> X86Feature::IsaAvx512
    77, // EvexVmovddupVpdWpd -> X86Feature::IsaAvx512
    77, // EvexVmovsldupVpsWps -> X86Feature::IsaAvx512
    77, // EvexVmovshdupVpsWps -> X86Feature::IsaAvx512
    77, // EvexVmovddupVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVmovsldupVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVmovshdupVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovqbWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovdbWdqVdq -> X86Feature::IsaAvx512
    79, // EvexVpmovwbWdqVdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovdwWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovqwWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovqdWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovqbWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovdbWdqVdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpmovwbWdqVdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovdwWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovqwWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovqdWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovusqbWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovusdbWdqVdq -> X86Feature::IsaAvx512
    79, // EvexVpmovuswbWdqVdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovusdwWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovusqwWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovusqdWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovusqbWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovusdbWdqVdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpmovuswbWdqVdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovusdwWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovusqwWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovusqdWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsqbWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsdbWdqVdq -> X86Feature::IsaAvx512
    79, // EvexVpmovswbWdqVdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovsdwWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsqwWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsqdWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsqbWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsdbWdqVdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpmovswbWdqVdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovsdwWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsqwWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsqdWdqVdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpmovsxbwVdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovsxbdVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsxbqVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsxwdVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsxwqVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovsxdqVdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpmovsxbwVdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovsxbdVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsxbqVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsxwdVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsxwqVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovsxdqVdqWdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpmovzxbwVdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovzxbdVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovzxbqVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovzxwdVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovzxwqVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmovzxdqVdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpmovzxbwVdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpmovzxbdVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovzxbqVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovzxwdVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovzxwqVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmovzxdqVdqWdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpsubbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubsbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubusbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubuswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddsbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddusbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpadduswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubsbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubusbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsubuswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddsbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddusbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpaddswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpadduswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpminsbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpminubVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxubVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxsbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpminswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpminuwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxuwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpminsbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpminubVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxubVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxsbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpminswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpminuwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaxuwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpacksswbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpacksswbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpackuswbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpackuswbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpackssdwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpackssdwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpackusdwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpackusdwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpcklbwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpckhbwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpcklbwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpckhbwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpcklwdVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpckhwdVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpcklwdVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpunpckhwdVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpavgbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpavgwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpavgbVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpavgwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaddubswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaddubswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmullwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmulhwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmulhuwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmulhrswVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmullwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmulhwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmulhuwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpmulhrswVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrldqUdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpslldqUdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpsadbwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaddwdVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmaddwdVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    83, // EvexVpmadd52luqVdqHdqWdq -> X86Feature::IsaAvx512Ifma52
    83, // EvexVpmadd52luqVdqHdqWdqKmask -> X86Feature::IsaAvx512Ifma52
    83, // EvexVpmadd52huqVdqHdqWdq -> X86Feature::IsaAvx512Ifma52
    83, // EvexVpmadd52huqVdqHdqWdqKmask -> X86Feature::IsaAvx512Ifma52
    ISA_ALWAYS, // EvexVpmultishiftqbVdqHdqWdq
    81, // EvexVpmultishiftqbVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi
    81, // EvexVpermbVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi
    79, // EvexVpermwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    81, // EvexVpermt2bVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi
    79, // EvexVpermt2wVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    81, // EvexVpermi2bVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi
    79, // EvexVpermi2wVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVinsertf32x4VpsHpsWpsIb -> X86Feature::IsaAvx512
    78, // EvexVinsertf64x2VpdHpdWpdIb -> X86Feature::IsaAvx512Dq
    77, // EvexVinsertf32x4VpsHpsWpsIbKmask -> X86Feature::IsaAvx512
    78, // EvexVinsertf64x2VpdHpdWpdIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVinsertf32x8VpsHpsWpsIb -> X86Feature::IsaAvx512Dq
    77, // EvexVinsertf64x4VpdHpdWpdIb -> X86Feature::IsaAvx512
    78, // EvexVinsertf32x8VpsHpsWpsIbKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVinsertf64x4VpdHpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVinserti32x4VdqHdqWdqIb -> X86Feature::IsaAvx512
    78, // EvexVinserti64x2VdqHdqWdqIb -> X86Feature::IsaAvx512Dq
    77, // EvexVinserti32x4VdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    78, // EvexVinserti64x2VdqHdqWdqIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVinserti32x8VdqHdqWdqIb -> X86Feature::IsaAvx512Dq
    77, // EvexVinserti64x4VdqHdqWdqIb -> X86Feature::IsaAvx512
    78, // EvexVinserti32x8VdqHdqWdqIbKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVinserti64x4VdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVextractf32x4WpsVpsIb -> X86Feature::IsaAvx512
    78, // EvexVextractf64x2WpdVpdIb -> X86Feature::IsaAvx512Dq
    77, // EvexVextractf32x4WpsVpsIbKmask -> X86Feature::IsaAvx512
    78, // EvexVextractf64x2WpdVpdIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVextractf32x8WpsVpsIb -> X86Feature::IsaAvx512Dq
    77, // EvexVextractf64x4WpdVpdIb -> X86Feature::IsaAvx512
    78, // EvexVextractf32x8WpsVpsIbKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVextractf64x4WpdVpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVextracti32x4WdqVdqIb -> X86Feature::IsaAvx512
    78, // EvexVextracti64x2WdqVdqIb -> X86Feature::IsaAvx512Dq
    77, // EvexVextracti32x4WdqVdqIbKmask -> X86Feature::IsaAvx512
    78, // EvexVextracti64x2WdqVdqIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVextracti32x8WdqVdqIb -> X86Feature::IsaAvx512Dq
    77, // EvexVextracti64x4WdqVdqIb -> X86Feature::IsaAvx512
    78, // EvexVextracti32x8WdqVdqIbKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVextracti64x4WdqVdqIbKmask -> X86Feature::IsaAvx512
    78, // EvexVbroadcastf32x2VpsWq -> X86Feature::IsaAvx512Dq
    78, // EvexVbroadcastf32x2VpsWqKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVbroadcasti32x2VdqWq -> X86Feature::IsaAvx512Dq
    78, // EvexVbroadcasti32x2VdqWqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcastf32x4VpsWps -> X86Feature::IsaAvx512
    78, // EvexVbroadcastf64x2VpdWpd -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcastf32x4VpsWpsKmask -> X86Feature::IsaAvx512
    78, // EvexVbroadcastf64x2VpdWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVbroadcastf32x8VpsWps -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcastf64x4VpdWpd -> X86Feature::IsaAvx512
    78, // EvexVbroadcastf32x8VpsWpsKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcastf64x4VpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVbroadcasti32x4VdqWdq -> X86Feature::IsaAvx512
    78, // EvexVbroadcasti64x2VdqWdq -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcasti32x4VdqWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVbroadcasti64x2VdqWdqKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVbroadcasti32x8VdqWdq -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcasti64x4VdqWdq -> X86Feature::IsaAvx512
    78, // EvexVbroadcasti32x8VdqWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVbroadcasti64x4VdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmulldVdqHdqWdq -> X86Feature::IsaAvx512
    78, // EvexVpmullqVdqHdqWdq -> X86Feature::IsaAvx512Dq
    77, // EvexVpmulldVdqHdqWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVpmullqVdqHdqWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVpadddVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpaddqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpadddVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpaddqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsubdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsubqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsubdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsubqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpanddVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpandqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpanddVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpandqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpandndVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpandnqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpandndVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpandnqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpordVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVporqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpordVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVporqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpxordVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpxorqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpxordVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpxorqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVandpsVpsHpsWps -> X86Feature::IsaAvx512Dq
    78, // EvexVandpdVpdHpdWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVandpsVpsHpsWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVandpdVpdHpdWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVandnpsVpsHpsWps -> X86Feature::IsaAvx512Dq
    78, // EvexVandnpdVpdHpdWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVandnpsVpsHpsWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVandnpdVpdHpdWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVorpsVpsHpsWps -> X86Feature::IsaAvx512Dq
    78, // EvexVorpdVpdHpdWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVorpsVpsHpsWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVorpdVpdHpdWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVxorpsVpsHpsWps -> X86Feature::IsaAvx512Dq
    78, // EvexVxorpdVpdHpdWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVxorpsVpsHpsWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVxorpdVpdHpdWpdKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVpmaxsdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmaxsqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmaxsdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmaxsqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmaxudVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmaxuqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpmaxudVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpmaxuqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpminsdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpminsqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpminsdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpminsqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpminudVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpminuqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpminudVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpminuqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexValigndVdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexValignqVdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    79, // EvexVpalignrVdqHdqWdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpalignrVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVdbpsadbwVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVpsrlvwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpsrlvdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsrlvqVdqHdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpsravwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpsravdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsravqVdqHdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpsllvwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVpsllvdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsllvqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVprolvdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVprolvqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVprorvdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVprorvqVdqHdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpsrlvwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpsrlvdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsrlvqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpsravwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpsravdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsravqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    79, // EvexVpsllvwVdqHdqWdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVpsllvdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsllvqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVprolvdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVprolvqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVprorvdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVprorvqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsradVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsraqVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpsradVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsraqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpsradUdqIb -> X86Feature::IsaAvx512
    77, // EvexVpsraqUdqIb -> X86Feature::IsaAvx512
    77, // EvexVprordUdqIb -> X86Feature::IsaAvx512
    77, // EvexVprorqUdqIb -> X86Feature::IsaAvx512
    77, // EvexVproldUdqIb -> X86Feature::IsaAvx512
    77, // EvexVprolqUdqIb -> X86Feature::IsaAvx512
    77, // EvexVpsradUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpsraqUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVprordUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVprorqUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVproldUdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVprolqUdqIbKmask -> X86Feature::IsaAvx512
    79, // EvexVmovdqu8VdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu16VdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu8VdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu16VdqWdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu8WdqVdq -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu16WdqVdq -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu8WdqVdqKmask -> X86Feature::IsaAvx512Bw
    79, // EvexVmovdqu16WdqVdqKmask -> X86Feature::IsaAvx512Bw
    77, // EvexVmovdqu32VdqWdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqu64VdqWdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqu32VdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqu64VdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqu32WdqVdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqu64WdqVdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqu32WdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqu64WdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqa32VdqWdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqa64VdqWdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqa32VdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqa64VdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqa32WdqVdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqa64WdqVdq -> X86Feature::IsaAvx512
    77, // EvexVmovdqa32WdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVmovdqa64WdqVdqKmask -> X86Feature::IsaAvx512
    78, // EvexVrangepsVpsHpsWpsIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVrangepdVpdHpdWpdIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVrangessVssHpsWssIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVrangesdVsdHpdWsdIbKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVgetexppsVpsWps -> X86Feature::IsaAvx512
    77, // EvexVgetexppdVpdWpd -> X86Feature::IsaAvx512
    77, // EvexVgetexpssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVgetexpsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVgetexppsVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVgetexppdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVgetexpssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVgetexpsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVgetmantpsVpsWpsIbKmask -> X86Feature::IsaAvx512
    77, // EvexVgetmantpdVpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVgetmantssVssHpsWssIbKmask -> X86Feature::IsaAvx512
    77, // EvexVgetmantsdVsdHpdWsdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVscalefpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVscalefpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVscalefssVssHpsWss -> X86Feature::IsaAvx512
    77, // EvexVscalefsdVsdHpdWsd -> X86Feature::IsaAvx512
    77, // EvexVscalefpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVscalefpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVscalefssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVscalefsdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVrcp14psVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVrcp14pdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVrcp14ssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVrcp14sdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVrsqrt14psVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVrsqrt14pdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVrsqrt14ssVssHpsWssKmask -> X86Feature::IsaAvx512
    77, // EvexVrsqrt14sdVsdHpdWsdKmask -> X86Feature::IsaAvx512
    78, // EvexVcvtps2uqqVdqWps -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtpd2uqqVdqWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtps2uqqVdqWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtpd2uqqVdqWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttps2uqqVdqWps -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttps2uqqVdqWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttpd2uqqVdqWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttpd2uqqVdqWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtps2qqVdqWps -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtps2qqVdqWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtpd2qqVdqWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtpd2qqVdqWpdKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttps2qqVdqWps -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttps2qqVdqWpsKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttpd2qqVdqWpd -> X86Feature::IsaAvx512Dq
    78, // EvexVcvttpd2qqVdqWpdKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVcvttps2udqVdqWps -> X86Feature::IsaAvx512
    77, // EvexVcvttpd2udqVdqWpd -> X86Feature::IsaAvx512
    77, // EvexVcvttps2udqVdqWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcvttpd2udqVdqWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtps2udqVdqWps -> X86Feature::IsaAvx512
    77, // EvexVcvtpd2udqVdqWpd -> X86Feature::IsaAvx512
    77, // EvexVcvtps2udqVdqWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtpd2udqVdqWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVcvtudq2pdVpdWdq -> X86Feature::IsaAvx512
    77, // EvexVcvtudq2pdVpdWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVcvtuqq2pdVpdWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtuqq2pdVpdWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVcvtudq2psVpsWdq -> X86Feature::IsaAvx512
    77, // EvexVcvtudq2psVpsWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVcvtuqq2psVpsWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtuqq2psVpsWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVcvtdq2pdVpdWdq -> X86Feature::IsaAvx512
    77, // EvexVcvtdq2pdVpdWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVcvtqq2pdVpdWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtqq2pdVpdWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVcvtdq2psVpsWdq -> X86Feature::IsaAvx512
    77, // EvexVcvtdq2psVpsWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVcvtqq2psVpsWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtqq2psVpsWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVfmadd132psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmadd132pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmadd213psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmadd213pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmadd231psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmadd231pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmadd132psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd132pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd213psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd213pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd231psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd231pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd132ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfmadd132sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfmadd213ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfmadd213sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfmadd231ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfmadd231sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfmadd132ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd132sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd213ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd213sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd231ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfmadd231sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub132psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub132pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub213psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub213pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub231psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub231pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub132psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub132pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub213psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub213pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub231psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmaddsub231pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd132psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd132pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd213psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd213pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd231psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd231pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd132psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd132pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd213psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd213pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd231psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsubadd231pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub132psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmsub132pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmsub213psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmsub213pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmsub231psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfmsub231pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfmsub132psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub132pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub213psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub213pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub231psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub231pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub132ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfmsub132sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfmsub213ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfmsub213sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfmsub231ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfmsub231sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfmsub132ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub132sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub213ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub213sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub231ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfmsub231sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd132sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd213sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmadd231sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231psVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231pdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231ssVpsHssWss -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231sdVpdHsdWsd -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub132sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub213sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231ssVpsHssWssKmask -> X86Feature::IsaAvx512
    77, // EvexVfnmsub231sdVpdHsdWsdKmask -> X86Feature::IsaAvx512
    77, // EvexVpcmpbKgqHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpwKgdHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpubKgqHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpuwKgdHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpdKgwHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpqKgbHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpudKgwHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpcmpuqKgbHdqWdqIb -> X86Feature::IsaAvx512
    79, // EvexVptestmbKgqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVptestmwKgdHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVptestnmbKgqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVptestnmwKgdHdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVptestmdKgwHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVptestmqKgbHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVptestnmdKgwHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVptestnmqKgbHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpternlogdVdqHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpternlogqVdqHdqWdqIb -> X86Feature::IsaAvx512
    77, // EvexVpternlogdVdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVpternlogqVdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVgatherdpsVpsVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherdpdVpdVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherqpsVpsVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherqpdVpdVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherddVdqVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherdqVdqVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherqdVdqVsib -> X86Feature::IsaAvx512
    77, // EvexVgatherqqVdqVsib -> X86Feature::IsaAvx512
    77, // EvexVscatterdpsVsibVps -> X86Feature::IsaAvx512
    77, // EvexVscatterdpdVsibVpd -> X86Feature::IsaAvx512
    77, // EvexVscatterqpsVsibVps -> X86Feature::IsaAvx512
    77, // EvexVscatterqpdVsibVpd -> X86Feature::IsaAvx512
    77, // EvexVscatterddVsibVdq -> X86Feature::IsaAvx512
    77, // EvexVscatterdqVsibVdq -> X86Feature::IsaAvx512
    77, // EvexVscatterqdVsibVdq -> X86Feature::IsaAvx512
    77, // EvexVscatterqqVsibVdq -> X86Feature::IsaAvx512
    77, // EvexVblendmpsVpsHpsWps -> X86Feature::IsaAvx512
    77, // EvexVblendmpdVpdHpdWpd -> X86Feature::IsaAvx512
    77, // EvexVpblendmdVdqHdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpblendmqVdqHdqWdq -> X86Feature::IsaAvx512
    79, // EvexVpblendmbVdqHdqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpblendmwVdqHdqWdq -> X86Feature::IsaAvx512Bw
    77, // EvexVshufi32x4VdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVshufi64x2VdqHdqWdqIbKmask -> X86Feature::IsaAvx512
    77, // EvexVshuff32x4VpsHpsWpsIbKmask -> X86Feature::IsaAvx512
    77, // EvexVshuff64x2VpdHpdWpdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVexpandpsVpsWps -> X86Feature::IsaAvx512
    77, // EvexVexpandpdVpdWpd -> X86Feature::IsaAvx512
    77, // EvexVexpandpsVpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVexpandpdVpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVcompresspsWpsVps -> X86Feature::IsaAvx512
    77, // EvexVcompresspdWpdVpd -> X86Feature::IsaAvx512
    77, // EvexVcompresspsWpsVpsKmask -> X86Feature::IsaAvx512
    77, // EvexVcompresspdWpdVpdKmask -> X86Feature::IsaAvx512
    82, // EvexVpexpandbVdqWdq -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpexpandwVdqWdq -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpexpandbVdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpexpandwVdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    77, // EvexVpexpanddVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpexpandqVdqWdq -> X86Feature::IsaAvx512
    77, // EvexVpexpanddVdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpexpandqVdqWdqKmask -> X86Feature::IsaAvx512
    82, // EvexVpcompressbWdqVdq -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpcompresswWdqVdq -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpcompressbWdqVdqKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpcompresswWdqVdqKmask -> X86Feature::IsaAvx512Vbmi2
    77, // EvexVpcompressdWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpcompressqWdqVdq -> X86Feature::IsaAvx512
    77, // EvexVpcompressdWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpcompressqWdqVdqKmask -> X86Feature::IsaAvx512
    77, // EvexVfixupimmssVssHssWssIbKmask -> X86Feature::IsaAvx512
    77, // EvexVfixupimmsdVsdHsdWsdIbKmask -> X86Feature::IsaAvx512
    77, // EvexVfixupimmpsVpsHpsWpsIb -> X86Feature::IsaAvx512
    77, // EvexVfixupimmpdVpdHpdWpdIb -> X86Feature::IsaAvx512
    77, // EvexVfixupimmpsVpsHpsWpsIbKmask -> X86Feature::IsaAvx512
    77, // EvexVfixupimmpdVpdHpdWpdIbKmask -> X86Feature::IsaAvx512
    78, // EvexVfpclasspsKgwWpsIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVfpclasspdKgbWpdIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVfpclassssKgbWssIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVfpclasssdKgbWsdIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVreducepsVpsWpsIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVreducepdVpdWpdIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVreducessVssHpsWssIbKmask -> X86Feature::IsaAvx512Dq
    78, // EvexVreducesdVsdHpdWsdIbKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVpermt2dVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpermt2qVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpermi2dVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpermi2qVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpermt2psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVpermt2pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVpermi2psVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVpermi2pdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    77, // EvexVpermdVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpermqVdqHdqWdqKmask -> X86Feature::IsaAvx512
    77, // EvexVpermpsVpsHpsWpsKmask -> X86Feature::IsaAvx512
    77, // EvexVpermpdVpdHpdWpdKmask -> X86Feature::IsaAvx512
    80, // EvexVpconflictdVdqWdqKmask -> X86Feature::IsaAvx512Cd
    80, // EvexVpconflictqVdqWdqKmask -> X86Feature::IsaAvx512Cd
    80, // EvexVplzcntdVdqWdqKmask -> X86Feature::IsaAvx512Cd
    80, // EvexVplzcntqVdqWdqKmask -> X86Feature::IsaAvx512Cd
    79, // EvexVpmovm2bVdqKeq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmovm2wVdqKed -> X86Feature::IsaAvx512Bw
    78, // EvexVpmovm2dVdqKew -> X86Feature::IsaAvx512Dq
    78, // EvexVpmovm2qVdqKeb -> X86Feature::IsaAvx512Dq
    79, // EvexVpmovb2mKgqWdq -> X86Feature::IsaAvx512Bw
    79, // EvexVpmovw2mKgdWdq -> X86Feature::IsaAvx512Bw
    78, // EvexVpmovd2mKgwWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVpmovq2mKgbWdq -> X86Feature::IsaAvx512Dq
    86, // EvexVpopcntbVdqWdqKmask -> X86Feature::IsaAvx512Bitalg
    86, // EvexVpopcntwVdqWdqKmask -> X86Feature::IsaAvx512Bitalg
    84, // EvexVpopcntdVdqWdqKmask -> X86Feature::IsaAvx512Vpopcntdq
    84, // EvexVpopcntqVdqWdqKmask -> X86Feature::IsaAvx512Vpopcntdq
    82, // EvexVpshrddVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshrdqVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshrdvdVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshrdvqVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshlddVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshldqVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshldvdVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshldvqVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    77, // EvexVcvtss2siGdWss -> X86Feature::IsaAvx512
    77, // EvexVcvtss2siGqWss -> X86Feature::IsaAvx512
    77, // EvexVcvtsd2siGdWsd -> X86Feature::IsaAvx512
    77, // EvexVcvtsd2siGqWsd -> X86Feature::IsaAvx512
    77, // EvexVcvttss2siGdWss -> X86Feature::IsaAvx512
    77, // EvexVcvttss2siGqWss -> X86Feature::IsaAvx512
    77, // EvexVcvttsd2siGdWsd -> X86Feature::IsaAvx512
    77, // EvexVcvttsd2siGqWsd -> X86Feature::IsaAvx512
    77, // EvexVmovdVdqEd -> X86Feature::IsaAvx512
    77, // EvexVmovqVdqEq -> X86Feature::IsaAvx512
    77, // EvexVmovdEdVd -> X86Feature::IsaAvx512
    77, // EvexVmovqEqVq -> X86Feature::IsaAvx512
    77, // EvexVcvtsi2ssVssEd -> X86Feature::IsaAvx512
    77, // EvexVcvtsi2ssVssEq -> X86Feature::IsaAvx512
    77, // EvexVcvtsi2sdVsdEd -> X86Feature::IsaAvx512
    77, // EvexVcvtsi2sdVsdEq -> X86Feature::IsaAvx512
    77, // EvexVcvtusi2ssVssEd -> X86Feature::IsaAvx512
    77, // EvexVcvtusi2ssVssEq -> X86Feature::IsaAvx512
    77, // EvexVcvtusi2sdVsdEd -> X86Feature::IsaAvx512
    77, // EvexVcvtusi2sdVsdEq -> X86Feature::IsaAvx512
    77, // EvexVcvtss2usiGdWss -> X86Feature::IsaAvx512
    77, // EvexVcvtss2usiGqWss -> X86Feature::IsaAvx512
    77, // EvexVcvtsd2usiGdWsd -> X86Feature::IsaAvx512
    77, // EvexVcvtsd2usiGqWsd -> X86Feature::IsaAvx512
    77, // EvexVcvttss2usiGdWss -> X86Feature::IsaAvx512
    77, // EvexVcvttss2usiGqWss -> X86Feature::IsaAvx512
    77, // EvexVcvttsd2usiGdWsd -> X86Feature::IsaAvx512
    77, // EvexVcvttsd2usiGqWsd -> X86Feature::IsaAvx512
    79, // EvexVpinsrbVdqEbIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpinsrwVdqEwIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpextrwGdUdqIb -> X86Feature::IsaAvx512Bw
    79, // EvexVpextrbEdVdqIbR -> X86Feature::IsaAvx512Bw
    79, // EvexVpextrbMbVdqIbM -> X86Feature::IsaAvx512Bw
    79, // EvexVpextrwEdVdqIbR -> X86Feature::IsaAvx512Bw
    79, // EvexVpextrwMwVdqIbM -> X86Feature::IsaAvx512Bw
    78, // EvexVpinsrdVdqEdIb -> X86Feature::IsaAvx512Dq
    78, // EvexVpinsrqVdqEqIb -> X86Feature::IsaAvx512Dq
    78, // EvexVpextrdEdVdqIb -> X86Feature::IsaAvx512Dq
    78, // EvexVpextrqEqVdqIb -> X86Feature::IsaAvx512Dq
    77, // EvexVpbroadcastmb2qVdqKeb -> X86Feature::IsaAvx512
    77, // EvexVpbroadcastmw2dVdqKew -> X86Feature::IsaAvx512
    85, // EvexVpdpbusdVdqHdqWdq -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpbusdsVdqHdqWdq -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpwssdVdqHdqWdq -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpwssdsVdqHdqWdq -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpbusdVdqHdqWdqKmask -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpbusdsVdqHdqWdqKmask -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpwssdVdqHdqWdqKmask -> X86Feature::IsaAvx512Vnni
    85, // EvexVpdpwssdsVdqHdqWdqKmask -> X86Feature::IsaAvx512Vnni
    86, // EvexVpshufbitqmbKgqHdqWdqKmask -> X86Feature::IsaAvx512Bitalg
    87, // EvexVp2intersectdKgqHdqWdq -> X86Feature::IsaAvx512Vp2intersect
    87, // EvexVp2intersectqKgqHdqWdq -> X86Feature::IsaAvx512Vp2intersect
    82, // EvexVpshrdwVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshrdvwVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshldwVdqHdqWdqIbKmask -> X86Feature::IsaAvx512Vbmi2
    82, // EvexVpshldvwVdqHdqWdqKmask -> X86Feature::IsaAvx512Vbmi2
    89, // EvexVaddshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVaddshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVsubshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVsubshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmulshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVmulshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVdivshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVdivshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVminshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVminshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmaxshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVmaxshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVscalefshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVscalefshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVaddphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVaddphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVsubphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVsubphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmulphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVmulphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVdivphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVdivphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVminphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVminphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmaxphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVmaxphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVscalefphVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVscalefphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd132shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd132shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd213shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd213shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd231shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd231shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd132shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd132shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd213shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd213shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd231shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd231shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub132shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub132shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub213shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub213shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub231shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub231shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub132shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub132shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub213shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub213shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub231shVphHshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub231shVphHshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd132phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd132phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd213phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd213phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd231phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmadd231phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd132phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd132phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd213phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd213phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd231phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmadd231phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub132phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub132phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub213phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub213phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub231phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsub231phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub132phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub132phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub213phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub213phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub231phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfnmsub231phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddsub132phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddsub132phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddsub213phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddsub213phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddsub231phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddsub231phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsubadd132phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsubadd132phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsubadd213phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsubadd213phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsubadd231phVphHphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmsubadd231phVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfpclassphKgdWphIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfpclassshKgbWshIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVucomishVshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcomishVshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcmpphKgdHphWphIb -> X86Feature::IsaAvx512Fp16
    89, // EvexVcmpshKgbHshWshIb -> X86Feature::IsaAvx512Fp16
    89, // EvexVsqrtphVphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVsqrtphVphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVsqrtshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVsqrtshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVgetexpphVphWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVgetexpphVphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVgetexpshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVgetexpshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshVshWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshWshVsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshVshWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshWshVshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshVshHphWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshWshHphVsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovshWshHphVshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovwVshEw -> X86Feature::IsaAvx512Fp16
    89, // EvexVmovwEdVsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2uwVdqWps -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2uwVdqWpsKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2wVdqWps -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2wVdqWpsKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2uwVdqWps -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2uwVdqWpsKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2wVdqWps -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2wVdqWpsKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtuw2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtuw2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtw2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtw2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2psxVpsWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2psxVpsWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2dqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2dqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2udqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2udqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2dqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2dqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2udqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2udqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2pdVpdWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2pdVpdWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2qqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2qqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2uqqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtph2uqqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2qqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2qqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2uqqVdqWph -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttph2uqqVdqWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtps2phxVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtps2phxVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtdq2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtdq2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtudq2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtudq2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtpd2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtpd2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtqq2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtqq2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtuqq2phVphWdq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtuqq2phVphWdqKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2ssVssWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2ssVssWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2sdVsdWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2sdVsdWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtss2shVssWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtss2shVssWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsd2shVssWsh -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsd2shVssWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2siGdWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2siGqWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2usiGdWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsh2usiGqWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttsh2siGdWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttsh2siGqWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttsh2usiGdWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvttsh2usiGqWss -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsi2shVshEd -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtsi2shVshEq -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtusi2shVshEd -> X86Feature::IsaAvx512Fp16
    89, // EvexVcvtusi2shVshEq -> X86Feature::IsaAvx512Fp16
    89, // EvexVgetmantphVphWphIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVgetmantshVshHphWshIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVreducephVphWphIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVreduceshVshHphWshIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVrndscalephVphWphIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVrndscaleshVshHphWshIbKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVrcpphVphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVrcpshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVrsqrtphVphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVrsqrtshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmulcshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfcmulcshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmulcphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfcmulcphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddcshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfcmaddcshVshHphWshKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfmaddcphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    89, // EvexVfcmaddcphVphHphWphKmask -> X86Feature::IsaAvx512Fp16
    43, // EvexVaesencVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // EvexVaesenclastVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // EvexVaesdecVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // EvexVaesdeclastVdqHdqWdq -> X86Feature::IsaVaesVpclmulqdq
    43, // EvexVpclmulqdqVdqHdqWdqIb -> X86Feature::IsaVaesVpclmulqdq
    69, // EvexVgf2p8affineqbVdqHdqWdqIbKmask -> X86Feature::IsaGfni
    69, // EvexVgf2p8affineinvqbVdqHdqWdqIbKmask -> X86Feature::IsaGfni
    69, // EvexVgf2p8mulbVdqHdqWdqKmask -> X86Feature::IsaGfni
    71, // EvexVsm4key4VdqHdqWdq -> X86Feature::IsaSm4
    71, // EvexVsm4rnds4VdqHdqWdq -> X86Feature::IsaSm4
    100, // EvexVucomxssVssWss -> X86Feature::IsaAvx10_2
    100, // EvexVcomxssVssWss -> X86Feature::IsaAvx10_2
    100, // EvexVucomxsdVsdWsd -> X86Feature::IsaAvx10_2
    100, // EvexVcomxsdVsdWsd -> X86Feature::IsaAvx10_2
    100, // EvexVucomxshVshWsh -> X86Feature::IsaAvx10_2
    100, // EvexVcomxshVshWsh -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbssdVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbssdsVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbsudVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbsudsVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbuudVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbuudsVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbssdVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbssdsVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbsudVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbsudsVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbuudVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpbuudsVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwsudVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwsudsVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwusdVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwusdsVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwuudVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwuudsVdqHdqWdq -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwsudVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwsudsVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwusdVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwusdsVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwuudVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVpdpwuudsVdqHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVmpsadbwVdqHdqWdqIb -> X86Feature::IsaAvx10_2
    100, // EvexVmpsadbwVdqHdqWdqIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVdpphpsVpsHdqWdqKmask -> X86Feature::IsaAvx10_2
    100, // EvexVaddbf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVaddbf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVsubbf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVsubbf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVdivbf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVdivbf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVmulbf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVmulbf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    ISA_ALWAYS, // EvexVminpbf16VphHphWph
    ISA_ALWAYS, // EvexVminpbf16VphHphWphKmask
    ISA_ALWAYS, // EvexVmaxpbf16VphHphWph
    ISA_ALWAYS, // EvexVmaxpbf16VphHphWphKmask
    100, // EvexVscalefpbf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVscalefpbf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVsqrtbf16VphWph -> X86Feature::IsaAvx10_2
    100, // EvexVsqrtbf16VphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVgetexppbf16VphWph -> X86Feature::IsaAvx10_2
    100, // EvexVgetexppbf16VphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfmadd132bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfmadd132bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfmadd213bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfmadd213bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfmadd231bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfmadd231bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfmsub132bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfmsub132bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfmsub213bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfmsub213bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfmsub231bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfmsub231bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfnmadd132bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfnmadd132bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfnmadd213bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfnmadd213bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfnmadd231bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfnmadd231bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfnmsub132bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfnmsub132bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfnmsub213bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfnmsub213bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfnmsub231bf16VphHphWph -> X86Feature::IsaAvx10_2
    100, // EvexVfnmsub231bf16VphHphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVfpclasspbf16KgdWphIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcmppbf16KgdHphWphIb -> X86Feature::IsaAvx10_2
    100, // EvexVcomisbf16VshWsh -> X86Feature::IsaAvx10_2
    100, // EvexVgetmantpbf16VphWphIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVreducebf16VphWphIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVrndscalebf16VphWphIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVrcppbf16VphWph -> X86Feature::IsaAvx10_2
    100, // EvexVrcppbf16VphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVrsqrtpbf16VphWph -> X86Feature::IsaAvx10_2
    100, // EvexVrsqrtpbf16VphWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxpsVpsHpsWpsIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxssVssHpsWssIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxpdVpdHpdWpdIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxsdVsdHpdWsdIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxphVphHphWphIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxshVshHphWshIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVminmaxbf16VphHphWphIbKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvt2ps2phxVphHpsWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2qqsVdqWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2qqsVdqWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2qqsVdqWpd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2qqsVdqWpdKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2uqqsVdqWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2uqqsVdqWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2uqqsVdqWpd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2uqqsVdqWpdKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2dqsVdqWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2dqsVdqWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2dqsVdqWpd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2dqsVdqWpdKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2udqsVdqWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2udqsVdqWpd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2udqsVdqWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttpd2udqsVdqWpdKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttss2sisGdWss -> X86Feature::IsaAvx10_2
    100, // EvexVcvttss2sisGqWss -> X86Feature::IsaAvx10_2
    100, // EvexVcvttsd2sisGdWsd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttsd2sisGqWsd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttss2usisGdWss -> X86Feature::IsaAvx10_2
    100, // EvexVcvttss2usisGqWss -> X86Feature::IsaAvx10_2
    100, // EvexVcvttsd2usisGdWsd -> X86Feature::IsaAvx10_2
    100, // EvexVcvttsd2usisGqWsd -> X86Feature::IsaAvx10_2
    100, // EvexVmovwVshWsh -> X86Feature::IsaAvx10_2
    100, // EvexVmovwWshVsh -> X86Feature::IsaAvx10_2
    100, // EvexVmovdVdWd -> X86Feature::IsaAvx10_2
    100, // EvexVmovdWdVd -> X86Feature::IsaAvx10_2
    100, // EvexVcvthf82phVphWf8Kmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2bf8Vf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2bf8sVf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvt2ph2bf8Vf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvt2ph2bf8sVf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbiasph2bf8Vf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbiasph2bf8sVf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2hf8Vf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2hf8sVf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvt2ph2hf8Vf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvt2ph2hf8sVf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbiasph2hf8Vf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbiasph2hf8sVf8hdqWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbf162ibsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbf162ibsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbf162iubsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvtbf162iubsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttbf162ibsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvttbf162ibsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttbf162iubsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvttbf162iubsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2ibsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2ibsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2iubsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvtph2iubsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttph2ibsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvttph2ibsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttph2iubsV8bWph -> X86Feature::IsaAvx10_2
    100, // EvexVcvttph2iubsV8bWphKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtps2ibsV8bWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvtps2ibsV8bWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvtps2iubsV8bWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvtps2iubsV8bWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2ibsV8bWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2ibsV8bWpsKmask -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2iubsV8bWps -> X86Feature::IsaAvx10_2
    100, // EvexVcvttps2iubsV8bWpsKmask -> X86Feature::IsaAvx10_2
    98, // EvexTilemovrowVdqTrmIb -> X86Feature::IsaAmxAvx512
    98, // EvexTilemovrowVdqTrmBd -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowd2psVpsTrmIb -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowd2psVpsTrmBd -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2phlVphTrmIb -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2phlVphTrmBd -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2phhVphTrmIb -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2phhVphTrmBd -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2bf16lVphTrmIb -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2bf16lVphTrmBd -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2bf16hVphTrmIb -> X86Feature::IsaAmxAvx512
    98, // EvexTcvtrowps2bf16hVphTrmBd -> X86Feature::IsaAmxAvx512
    101, // EvexVmovrsbVdqWdq -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrsbVdqWdqKmask -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrswVdqWdq -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrswVdqWdqKmask -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrsdVdqWdq -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrsdVdqWdqKmask -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrsqVdqWdq -> X86Feature::IsaAvx10_2Movrs
    101, // EvexVmovrsqVdqWdqKmask -> X86Feature::IsaAvx10_2Movrs
    ISA_ALWAYS, // NoAvxState
    ISA_ALWAYS, // NoEvexState
];

/// Feature required to execute `opcode`, or `ISA_ALWAYS` if ungated.
#[inline]
pub fn opcode_isa_feature(opcode: Opcode) -> u16 {
    OPCODE_ISA[opcode as usize]
}

/// Number of opcodes carrying a real feature gate. Asserted by tests so
/// that a silent regeneration drop is caught.
pub const GATED_OPCODE_COUNT: usize = 2908;

/// Number of `Opcode` variants the table was generated against. A
/// mismatch with the enum means the table needs regenerating.
pub const OPCODE_VARIANT_COUNT: usize = 3679;

// EVEX encoding restrictions — Bochs cpu/decoder/fetchdecode.h.
// `EVEX.b` means embedded broadcast on a memory operand and SAE /
// embedded rounding on a register operand; an opcode that supports
// neither must #UD rather than silently ignore the bit.
/// Opcode participates in the EVEX prepare checks at all.
pub const PREPARE_EVEX: u16 = 0x080;
/// `EVEX.b` with a register operand (SAE) is illegal for this opcode.
pub const PREPARE_EVEX_NO_SAE: u16 = 0x180;
/// `EVEX.b` with a memory operand (broadcast) is illegal for this opcode.
pub const PREPARE_EVEX_NO_BROADCAST: u16 = 0x280;

/// BX_PREPARE_EVEX* attribute bits per opcode, from field 10 of
/// `bx_define_opcode`.
// A `const` rather than a `static`: the EVEX decode path is a
// `const fn`, and const evaluation may read consts but not statics.
pub const OPCODE_EVEX_FLAGS: [u16; 3679] = [
    0x000, // IaError
    0x000, // InsertedOpcode
    0x000, // Aaa
    0x000, // Aad
    0x000, // Aam
    0x000, // Aas
    0x000, // Daa
    0x000, // Das
    0x000, // AdcEbGb
    0x000, // AndEbGb
    0x000, // AddEbGb
    0x000, // CmpEbGb
    0x000, // OrEbGb
    0x000, // SbbEbGb
    0x000, // SubEbGb
    0x000, // TestEbGb
    0x000, // XorEbGb
    0x000, // AdcEwGw
    0x000, // AddEwGw
    0x000, // AndEwGw
    0x000, // CmpEwGw
    0x000, // OrEwGw
    0x000, // SbbEwGw
    0x000, // SubEwGw
    0x000, // TestEwGw
    0x000, // XorEwGw
    0x000, // AdcEdGd
    0x000, // AddEdGd
    0x000, // AndEdGd
    0x000, // CmpEdGd
    0x000, // OrEdGd
    0x000, // SbbEdGd
    0x000, // SubEdGd
    0x000, // TestEdGd
    0x000, // XorEdGd
    0x000, // AdcAlib
    0x000, // AddAlib
    0x000, // AndAlib
    0x000, // CmpAlib
    0x000, // OrAlib
    0x000, // SbbAlib
    0x000, // SubAlib
    0x000, // TestAlib
    0x000, // XorAlib
    0x000, // AdcAxiw
    0x000, // AddAxiw
    0x000, // AndAxiw
    0x000, // CmpAxiw
    0x000, // OrAxiw
    0x000, // SbbAxiw
    0x000, // SubAxiw
    0x000, // TestAxiw
    0x000, // XorAxiw
    0x000, // AdcEaxid
    0x000, // AddEaxid
    0x000, // AndEaxid
    0x000, // CmpEaxid
    0x000, // OrEaxid
    0x000, // SbbEaxid
    0x000, // SubEaxid
    0x000, // TestEaxid
    0x000, // XorEaxid
    0x000, // AddEbIb
    0x000, // OrEbIb
    0x000, // AdcEbIb
    0x000, // SbbEbIb
    0x000, // AndEbIb
    0x000, // SubEbIb
    0x000, // XorEbIb
    0x000, // TestEbIb
    0x000, // CmpEbIb
    0x000, // AddEwIw
    0x000, // OrEwIw
    0x000, // AdcEwIw
    0x000, // SbbEwIw
    0x000, // AndEwIw
    0x000, // SubEwIw
    0x000, // XorEwIw
    0x000, // TestEwIw
    0x000, // CmpEwIw
    0x000, // AddEwsIb
    0x000, // OrEwsIb
    0x000, // AdcEwsIb
    0x000, // SbbEwsIb
    0x000, // AndEwsIb
    0x000, // SubEwsIb
    0x000, // XorEwsIb
    0x000, // TestEwsIb
    0x000, // CmpEwsIb
    0x000, // AddEdId
    0x000, // OrEdId
    0x000, // AdcEdId
    0x000, // SbbEdId
    0x000, // AndEdId
    0x000, // SubEdId
    0x000, // XorEdId
    0x000, // TestEdId
    0x000, // CmpEdId
    0x000, // AddEdsIb
    0x000, // OrEdsIb
    0x000, // AdcEdsIb
    0x000, // SbbEdsIb
    0x000, // AndEdsIb
    0x000, // SubEdsIb
    0x000, // XorEdsIb
    0x000, // TestEdsIb
    0x000, // CmpEdsIb
    0x000, // XorEwGwZeroIdiom
    0x000, // XorGwEwZeroIdiom
    0x000, // XorEdGdZeroIdiom
    0x000, // XorGdEdZeroIdiom
    0x000, // SubEwGwZeroIdiom
    0x000, // SubGwEwZeroIdiom
    0x000, // SubEdGdZeroIdiom
    0x000, // SubGdEdZeroIdiom
    0x000, // AddGbEb
    0x000, // OrGbEb
    0x000, // AdcGbEb
    0x000, // SbbGbEb
    0x000, // AndGbEb
    0x000, // SubGbEb
    0x000, // XorGbEb
    0x000, // CmpGbEb
    0x000, // AdcGwEw
    0x000, // AddGwEw
    0x000, // AndGwEw
    0x000, // CmpGwEw
    0x000, // OrGwEw
    0x000, // SbbGwEw
    0x000, // SubGwEw
    0x000, // XorGwEw
    0x000, // AdcGdEd
    0x000, // AddGdEd
    0x000, // AndGdEd
    0x000, // CmpGdEd
    0x000, // OrGdEd
    0x000, // SbbGdEd
    0x000, // SubGdEd
    0x000, // XorGdEd
    0x000, // IncEb
    0x000, // IncEw
    0x000, // IncEd
    0x000, // DecEb
    0x000, // DecEw
    0x000, // DecEd
    0x000, // BsfGwEw
    0x000, // BsrGwEw
    0x000, // BsfGdEd
    0x000, // BsrGdEd
    0x000, // BtcEwGw
    0x000, // BtrEwGw
    0x000, // BtsEwGw
    0x000, // BtcEdGd
    0x000, // BtrEdGd
    0x000, // BtsEdGd
    0x000, // BtcEwIb
    0x000, // BtrEwIb
    0x000, // BtsEwIb
    0x000, // BtcEdIb
    0x000, // BtrEdIb
    0x000, // BtsEdIb
    0x000, // BtEwIb
    0x000, // BtEdIb
    0x000, // BtEwGw
    0x000, // BtEdGd
    0x000, // BoundGwMa
    0x000, // BoundGdMa
    0x000, // ArplEwGw
    0x000, // CallEd
    0x000, // CallEw
    0x000, // CallJd
    0x000, // CallJw
    0x000, // CallfOp16Ap
    0x000, // CallfOp32Ap
    0x000, // CallfOp16Ep
    0x000, // CallfOp32Ep
    0x000, // Cbw
    0x000, // Cdq
    0x000, // Cwd
    0x000, // Cwde
    0x000, // Clc
    0x000, // Cld
    0x000, // Cli
    0x000, // Clts
    0x000, // Cmc
    0x000, // Hlt
    0x000, // Clflush
    0x000, // Clflushopt
    0x000, // Clwb
    0x000, // Clzero
    0x000, // EnterOp16IwIb
    0x000, // EnterOp32IwIb
    0x000, // LeaveOp16
    0x000, // LeaveOp32
    0x000, // ImulGdEd
    0x000, // ImulGdEdId
    0x000, // ImulGdEdsIb
    0x000, // ImulGwEw
    0x000, // ImulGwEwIw
    0x000, // ImulGwEwsIb
    0x000, // InAlDx
    0x000, // InAlib
    0x000, // InAxDx
    0x000, // InAxib
    0x000, // InEaxDx
    0x000, // InEaxib
    0x000, // OutDxAl
    0x000, // OutDxAx
    0x000, // OutDxEax
    0x000, // OutIbAl
    0x000, // OutIbAx
    0x000, // OutIbEax
    0x000, // IntIb
    0x000, // INT1
    0x000, // INT3
    0x000, // Int0
    0x000, // IretOp16
    0x000, // IretOp32
    0x000, // JmpEd
    0x000, // JmpEw
    0x000, // JmpJw
    0x000, // JmpJbw
    0x000, // JmpJd
    0x000, // JmpJbd
    0x000, // JmpfAp
    0x000, // JmpfOp16Ep
    0x000, // JmpfOp32Ep
    0x000, // JcxzJbw
    0x000, // JecxzJbd
    0x000, // LoopJbw
    0x000, // LoopeJbw
    0x000, // LoopneJbw
    0x000, // LoopJbd
    0x000, // LoopeJbd
    0x000, // LoopneJbd
    0x000, // JbJw
    0x000, // JbeJw
    0x000, // JlJw
    0x000, // JleJw
    0x000, // JnbJw
    0x000, // JnbeJw
    0x000, // JnlJw
    0x000, // JnleJw
    0x000, // JnoJw
    0x000, // JnpJw
    0x000, // JnsJw
    0x000, // JnzJw
    0x000, // JoJw
    0x000, // JpJw
    0x000, // JsJw
    0x000, // JzJw
    0x000, // JbJbw
    0x000, // JbeJbw
    0x000, // JlJbw
    0x000, // JleJbw
    0x000, // JnbJbw
    0x000, // JnbeJbw
    0x000, // JnlJbw
    0x000, // JnleJbw
    0x000, // JnoJbw
    0x000, // JnpJbw
    0x000, // JnsJbw
    0x000, // JnzJbw
    0x000, // JoJbw
    0x000, // JpJbw
    0x000, // JsJbw
    0x000, // JzJbw
    0x000, // JbJd
    0x000, // JbeJd
    0x000, // JlJd
    0x000, // JleJd
    0x000, // JnbJd
    0x000, // JnbeJd
    0x000, // JnlJd
    0x000, // JnleJd
    0x000, // JnoJd
    0x000, // JnpJd
    0x000, // JnsJd
    0x000, // JnzJd
    0x000, // JoJd
    0x000, // JpJd
    0x000, // JsJd
    0x000, // JzJd
    0x000, // JbJbd
    0x000, // JbeJbd
    0x000, // JlJbd
    0x000, // JleJbd
    0x000, // JnbJbd
    0x000, // JnbeJbd
    0x000, // JnlJbd
    0x000, // JnleJbd
    0x000, // JnoJbd
    0x000, // JnpJbd
    0x000, // JnsJbd
    0x000, // JnzJbd
    0x000, // JoJbd
    0x000, // JpJbd
    0x000, // JsJbd
    0x000, // JzJbd
    0x000, // Sahf
    0x000, // Lahf
    0x000, // LdsGdMp
    0x000, // LdsGwMp
    0x000, // LesGdMp
    0x000, // LesGwMp
    0x000, // LfsGdMp
    0x000, // LfsGwMp
    0x000, // LssGdMp
    0x000, // LssGwMp
    0x000, // LgsGdMp
    0x000, // LgsGwMp
    0x000, // LarGwEw
    0x000, // LslGwEw
    0x000, // LarGdEw
    0x000, // LslGdEw
    0x000, // LeaGdM
    0x000, // LeaGwM
    0x000, // SidtMs
    0x000, // LidtMs
    0x000, // SgdtMs
    0x000, // LgdtMs
    0x000, // SldtEw
    0x000, // LldtEw
    0x000, // StrEw
    0x000, // LtrEw
    0x000, // SmswEw
    0x000, // LmswEw
    0x000, // MovCr0rd
    0x000, // MovCr2rd
    0x000, // MovCr3rd
    0x000, // MovCr4rd
    0x000, // MovRdCr0
    0x000, // MovRdCr2
    0x000, // MovRdCr3
    0x000, // MovRdCr4
    0x000, // MovRdDd
    0x000, // MovDdRd
    0x000, // MovEbIb
    0x000, // MovEdId
    0x000, // MovEwIw
    0x000, // MovGbEb
    0x000, // MovEbGb
    0x000, // MovGwEw
    0x000, // MovEwGw
    0x000, // MovOp32GdEd
    0x000, // MovOp32EdGd
    0x000, // MovEwSw
    0x000, // MovSwEw
    0x000, // MovAlod
    0x000, // MovAxod
    0x000, // MovEaxod
    0x000, // MovOdAl
    0x000, // MovOdAx
    0x000, // MovOdEax
    0x000, // MovsxGdEb
    0x000, // MovsxGdEw
    0x000, // MovsxGwEb
    0x000, // MovzxGdEb
    0x000, // MovzxGdEw
    0x000, // MovzxGwEb
    0x000, // Nop
    0x000, // Pause
    0x000, // PopEw
    0x000, // PopEd
    0x000, // PopOp16Sw
    0x000, // PopOp32Sw
    0x000, // PopaOp16
    0x000, // PopaOp32
    0x000, // PopfFw
    0x000, // PopfFd
    0x000, // PushEw
    0x000, // PushEd
    0x000, // PushId
    0x000, // PushSIb32
    0x000, // PushIw
    0x000, // PushSIb16
    0x000, // PushOp16Sw
    0x000, // PushOp32Sw
    0x000, // PushaOp16
    0x000, // PushaOp32
    0x000, // PushfFw
    0x000, // PushfFd
    0x000, // RepCmpsbXbYb
    0x000, // RepCmpsdXdYd
    0x000, // RepCmpswXwYw
    0x000, // RepInsbYbDx
    0x000, // RepInsdYdDx
    0x000, // RepInswYwDx
    0x000, // RepLodsbAlxb
    0x000, // RepLodsdEaxxd
    0x000, // RepLodswAxxw
    0x000, // RepMovsbYbXb
    0x000, // RepMovsdYdXd
    0x000, // RepMovswYwXw
    0x000, // RepOutsbDxxb
    0x000, // RepOutsdDxxd
    0x000, // RepOutswDxxw
    0x000, // RepScasbAlyb
    0x000, // RepScasdEaxyd
    0x000, // RepScaswAxyw
    0x000, // RepStosbYbAl
    0x000, // RepStosdYdEax
    0x000, // RepStoswYwAx
    0x000, // RetfOp16
    0x000, // RetfOp16Iw
    0x000, // RetfOp32
    0x000, // RetfOp32Iw
    0x000, // RetOp16
    0x000, // RetOp16Iw
    0x000, // RetOp32
    0x000, // RetOp32Iw
    0x000, // NotEb
    0x000, // NegEb
    0x000, // NotEw
    0x000, // NegEw
    0x000, // NotEd
    0x000, // NegEd
    0x000, // RolEb
    0x000, // RorEb
    0x000, // RclEb
    0x000, // RcrEb
    0x000, // ShlEb
    0x000, // ShrEb
    0x000, // SarEb
    0x000, // RolEw
    0x000, // RorEw
    0x000, // RclEw
    0x000, // RcrEw
    0x000, // ShlEw
    0x000, // ShrEw
    0x000, // SarEw
    0x000, // RolEd
    0x000, // RorEd
    0x000, // RclEd
    0x000, // RcrEd
    0x000, // ShlEd
    0x000, // ShrEd
    0x000, // SarEd
    0x000, // RolEbIb
    0x000, // RorEbIb
    0x000, // RclEbIb
    0x000, // RcrEbIb
    0x000, // ShlEbIb
    0x000, // ShrEbIb
    0x000, // SarEbIb
    0x000, // RolEwIb
    0x000, // RorEwIb
    0x000, // RclEwIb
    0x000, // RcrEwIb
    0x000, // ShlEwIb
    0x000, // ShrEwIb
    0x000, // SarEwIb
    0x000, // RolEdIb
    0x000, // RorEdIb
    0x000, // RclEdIb
    0x000, // RcrEdIb
    0x000, // ShlEdIb
    0x000, // ShrEdIb
    0x000, // SarEdIb
    0x000, // RolEbI1
    0x000, // RorEbI1
    0x000, // RclEbI1
    0x000, // RcrEbI1
    0x000, // ShlEbI1
    0x000, // ShrEbI1
    0x000, // SarEbI1
    0x000, // RolEwI1
    0x000, // RorEwI1
    0x000, // RclEwI1
    0x000, // RcrEwI1
    0x000, // ShlEwI1
    0x000, // ShrEwI1
    0x000, // SarEwI1
    0x000, // RolEdI1
    0x000, // RorEdI1
    0x000, // RclEdI1
    0x000, // RcrEdI1
    0x000, // ShlEdI1
    0x000, // ShrEdI1
    0x000, // SarEdI1
    0x000, // SetbEb
    0x000, // SetbeEb
    0x000, // SetlEb
    0x000, // SetleEb
    0x000, // SetnbEb
    0x000, // SetnbeEb
    0x000, // SetnlEb
    0x000, // SetnleEb
    0x000, // SetnoEb
    0x000, // SetnpEb
    0x000, // SetnsEb
    0x000, // SetnzEb
    0x000, // SetoEb
    0x000, // SetpEb
    0x000, // SetsEb
    0x000, // SetzEb
    0x000, // ShldEdGd
    0x000, // ShldEdGdIb
    0x000, // ShldEwGw
    0x000, // ShldEwGwIb
    0x000, // ShrdEdGd
    0x000, // ShrdEdGdIb
    0x000, // ShrdEwGw
    0x000, // ShrdEwGwIb
    0x000, // Rsm
    0x000, // Salc
    0x000, // Stc
    0x000, // Std
    0x000, // Sti
    0x000, // MulAleb
    0x000, // ImulAleb
    0x000, // DivAleb
    0x000, // IdivAleb
    0x000, // MulAxew
    0x000, // ImulAxew
    0x000, // DivAxew
    0x000, // IdivAxew
    0x000, // MulEaxed
    0x000, // ImulEaxed
    0x000, // DivEaxed
    0x000, // IdivEaxed
    0x000, // VerrEw
    0x000, // VerwEw
    0x000, // XchgEbGb
    0x000, // XchgEwGw
    0x000, // XchgEdGd
    0x000, // XchgRxax
    0x000, // XchgErxEax
    0x000, // Xlat
    0x000, // Sysenter
    0x000, // Sysexit
    0x000, // Monitor
    0x000, // Mwait
    0x000, // UmonitorEq
    0x000, // UmonitorEd
    0x000, // UmwaitEd
    0x000, // TpauseEd
    0x000, // Monitorx
    0x000, // Mwaitx
    0x000, // Fwait
    0x000, // FldSti
    0x000, // FldSingleReal
    0x000, // FldDoubleReal
    0x000, // FldExtendedReal
    0x000, // FildWordInteger
    0x000, // FildDwordInteger
    0x000, // FildQwordInteger
    0x000, // FbldPackedBcd
    0x000, // FstSti
    0x000, // FstpSti
    0x000, // FstpSpecialSti
    0x000, // FstSingleReal
    0x000, // FstpSingleReal
    0x000, // FstDoubleReal
    0x000, // FstpDoubleReal
    0x000, // FstpExtendedReal
    0x000, // FistWordInteger
    0x000, // FistpWordInteger
    0x000, // FistDwordInteger
    0x000, // FistpDwordInteger
    0x000, // FistpQwordInteger
    0x000, // FbstpPackedBcd
    0x000, // FisttpMw
    0x000, // FisttpMd
    0x000, // FisttpMq
    0x000, // Fninit
    0x000, // Fnclex
    0x000, // Frstor
    0x000, // Fnsave
    0x000, // Fldenv
    0x000, // Fnstenv
    0x000, // Fldcw
    0x000, // Fnstcw
    0x000, // Fnstsw
    0x000, // FnstswAx
    0x000, // FLD1
    0x000, // Fldl2t
    0x000, // Fldl2e
    0x000, // Fldpi
    0x000, // Fldlg2
    0x000, // Fldln2
    0x000, // Fldz
    0x000, // FaddSt0Stj
    0x000, // FaddStiSt0
    0x000, // FaddpStiSt0
    0x000, // FaddSingleReal
    0x000, // FaddDoubleReal
    0x000, // FiaddWordInteger
    0x000, // FiaddDwordInteger
    0x000, // FmulSt0Stj
    0x000, // FmulStiSt0
    0x000, // FmulpStiSt0
    0x000, // FmulSingleReal
    0x000, // FmulDoubleReal
    0x000, // FimulWordInteger
    0x000, // FimulDwordInteger
    0x000, // FsubSt0Stj
    0x000, // FsubrSt0Stj
    0x000, // FsubStiSt0
    0x000, // FsubpStiSt0
    0x000, // FsubrStiSt0
    0x000, // FsubrpStiSt0
    0x000, // FsubSingleReal
    0x000, // FsubrSingleReal
    0x000, // FsubDoubleReal
    0x000, // FsubrDoubleReal
    0x000, // FisubWordInteger
    0x000, // FisubrWordInteger
    0x000, // FisubDwordInteger
    0x000, // FisubrDwordInteger
    0x000, // FdivSt0Stj
    0x000, // FdivrSt0Stj
    0x000, // FdivStiSt0
    0x000, // FdivpStiSt0
    0x000, // FdivrStiSt0
    0x000, // FdivrpStiSt0
    0x000, // FdivSingleReal
    0x000, // FdivrSingleReal
    0x000, // FdivDoubleReal
    0x000, // FdivrDoubleReal
    0x000, // FidivWordInteger
    0x000, // FidivrWordInteger
    0x000, // FidivDwordInteger
    0x000, // FidivrDwordInteger
    0x000, // FcomSti
    0x000, // FcompSti
    0x000, // FucomSti
    0x000, // FucompSti
    0x000, // FcomiSt0Stj
    0x000, // FcomipSt0Stj
    0x000, // FucomiSt0Stj
    0x000, // FucomipSt0Stj
    0x000, // FcomSingleReal
    0x000, // FcompSingleReal
    0x000, // FcomDoubleReal
    0x000, // FcompDoubleReal
    0x000, // FicomWordInteger
    0x000, // FicompWordInteger
    0x000, // FicomDwordInteger
    0x000, // FicompDwordInteger
    0x000, // FcmovbSt0Stj
    0x000, // FcmoveSt0Stj
    0x000, // FcmovbeSt0Stj
    0x000, // FcmovuSt0Stj
    0x000, // FcmovnbSt0Stj
    0x000, // FcmovneSt0Stj
    0x000, // FcmovnbeSt0Stj
    0x000, // FcmovnuSt0Stj
    0x000, // Fcompp
    0x000, // Fucompp
    0x000, // FxchSti
    0x000, // Fnop
    0x000, // Fplegacy
    0x000, // Fchs
    0x000, // Fabs
    0x000, // Ftst
    0x000, // Fxam
    0x000, // Fdecstp
    0x000, // Fincstp
    0x000, // FfreeSti
    0x000, // FfreepSti
    0x000, // F2XM1
    0x000, // FYL2X
    0x000, // Fptan
    0x000, // Fpatan
    0x000, // Fxtract
    0x000, // FPREM1
    0x000, // Fprem
    0x000, // FYL2XP1
    0x000, // Fsqrt
    0x000, // Fsincos
    0x000, // Frndint
    0x000, // Fscale
    0x000, // Fsin
    0x000, // Fcos
    0x000, // Fpuesc
    0x000, // Cpuid
    0x000, // BswapRx
    0x000, // BswapErx
    0x000, // Invd
    0x000, // Wbinvd
    0x000, // XaddEbGb
    0x000, // XaddEwGw
    0x000, // XaddEdGd
    0x000, // CmpxchgEbGb
    0x000, // CmpxchgEwGw
    0x000, // CmpxchgEdGd
    0x000, // Invlpg
    0x000, // Cmpxchg8b
    0x000, // Wrmsr
    0x000, // Rdmsr
    0x000, // Rdtsc
    0x000, // PunpcklbwPqQd
    0x000, // PunpcklwdPqQd
    0x000, // PunpckldqPqQd
    0x000, // PacksswbPqQq
    0x000, // PcmpgtbPqQq
    0x000, // PcmpgtwPqQq
    0x000, // PcmpgtdPqQq
    0x000, // PackuswbPqQq
    0x000, // PunpckhbwPqQq
    0x000, // PunpckhwdPqQq
    0x000, // PunpckhdqPqQq
    0x000, // PackssdwPqQq
    0x000, // MovdPqEd
    0x000, // MovqPqQq
    0x000, // PcmpeqbPqQq
    0x000, // PcmpeqwPqQq
    0x000, // PcmpeqdPqQq
    0x000, // Emms
    0x000, // MovdEdPq
    0x000, // MovqQqPq
    0x000, // PsrlwPqQq
    0x000, // PsrldPqQq
    0x000, // PsrlqPqQq
    0x000, // PmullwPqQq
    0x000, // PsubusbPqQq
    0x000, // PsubuswPqQq
    0x000, // PandPqQq
    0x000, // PaddusbPqQq
    0x000, // PadduswPqQq
    0x000, // PandnPqQq
    0x000, // PsrawPqQq
    0x000, // PsradPqQq
    0x000, // PmulhwPqQq
    0x000, // PsubsbPqQq
    0x000, // PsubswPqQq
    0x000, // PorPqQq
    0x000, // PaddsbPqQq
    0x000, // PaddswPqQq
    0x000, // PxorPqQq
    0x000, // PsllwPqQq
    0x000, // PslldPqQq
    0x000, // PsllqPqQq
    0x000, // PmaddwdPqQq
    0x000, // PsubbPqQq
    0x000, // PsubwPqQq
    0x000, // PsubdPqQq
    0x000, // PaddbPqQq
    0x000, // PaddwPqQq
    0x000, // PadddPqQq
    0x000, // PsrlwNqIb
    0x000, // PsrawNqIb
    0x000, // PsllwNqIb
    0x000, // PsrldNqIb
    0x000, // PsradNqIb
    0x000, // PslldNqIb
    0x000, // PsrlqNqIb
    0x000, // PsllqNqIb
    0x000, // MovqEqPq
    0x000, // Femms
    0x000, // Pf2idPqQq
    0x000, // Pf2iwPqQq
    0x000, // PfaccPqQq
    0x000, // PfaddPqQq
    0x000, // PfcmpeqPqQq
    0x000, // PfcmpgePqQq
    0x000, // PfcmpgtPqQq
    0x000, // PfmaxPqQq
    0x000, // PfminPqQq
    0x000, // PfmulPqQq
    0x000, // PfnaccPqQq
    0x000, // PfpnaccPqQq
    0x000, // PfrcpPqQq
    0x000, // Pfrcpit1PqQq
    0x000, // Pfrcpit2PqQq
    0x000, // Pfrsqit1PqQq
    0x000, // PfrsqrtPqQq
    0x000, // PfsubPqQq
    0x000, // PfsubrPqQq
    0x000, // Pi2fdPqQq
    0x000, // Pi2fwPqQq
    0x000, // PmulhrwPqQq
    0x000, // PswapdPqQq
    0x000, // PrefetchwMb
    0x000, // SyscallLegacy
    0x000, // SysretLegacy
    0x000, // CmovbGwEw
    0x000, // CmovbeGwEw
    0x000, // CmovlGwEw
    0x000, // CmovleGwEw
    0x000, // CmovnbGwEw
    0x000, // CmovnbeGwEw
    0x000, // CmovnlGwEw
    0x000, // CmovnleGwEw
    0x000, // CmovnoGwEw
    0x000, // CmovnpGwEw
    0x000, // CmovnsGwEw
    0x000, // CmovnzGwEw
    0x000, // CmovoGwEw
    0x000, // CmovpGwEw
    0x000, // CmovsGwEw
    0x000, // CmovzGwEw
    0x000, // CmovbGdEd
    0x000, // CmovbeGdEd
    0x000, // CmovlGdEd
    0x000, // CmovleGdEd
    0x000, // CmovnbGdEd
    0x000, // CmovnbeGdEd
    0x000, // CmovnlGdEd
    0x000, // CmovnleGdEd
    0x000, // CmovnoGdEd
    0x000, // CmovnpGdEd
    0x000, // CmovnsGdEd
    0x000, // CmovnzGdEd
    0x000, // CmovoGdEd
    0x000, // CmovpGdEd
    0x000, // CmovsGdEd
    0x000, // CmovzGdEd
    0x000, // Rdpmc
    0x000, // Ud0
    0x000, // Ud1
    0x000, // Ud2
    0x000, // Fxsave
    0x000, // Fxrstor
    0x000, // Ldmxcsr
    0x000, // Stmxcsr
    0x000, // PrefetchMb
    0x000, // Prefetcht0Mb
    0x000, // Prefetcht1Mb
    0x000, // Prefetcht2Mb
    0x000, // PrefetchntaMb
    0x000, // AndpsVpsWps
    0x000, // OrpsVpsWps
    0x000, // XorpsVpsWps
    0x000, // AndnpsVpsWps
    0x000, // MovupsVpsWps
    0x000, // MovupsWpsVps
    0x000, // MovssVssWss
    0x000, // MovssWssVss
    0x000, // MovlpsVpsMq
    0x000, // MovhlpsVpsWps
    0x000, // MovlpsMqVps
    0x000, // MovhpsVpsMq
    0x000, // MovlhpsVpsWps
    0x000, // MovhpsMqVps
    0x000, // MovapsVpsWps
    0x000, // MovapsWpsVps
    0x000, // MovntpsMpsVps
    0x000, // Cvtpi2psVpsQq
    0x000, // Cvtsi2ssVssEd
    0x000, // Cvttps2piPqWps
    0x000, // Cvtps2piPqWps
    0x000, // Cvttss2siGdWss
    0x000, // Cvtss2siGdWss
    0x000, // UcomissVssWss
    0x000, // ComissVssWss
    0x000, // MovmskpsGdUps
    0x000, // MovmskpdGdUpd
    0x000, // RsqrtpsVpsWps
    0x000, // RsqrtssVssWss
    0x000, // RcppsVpsWps
    0x000, // RcpssVssWss
    0x000, // PshufwPqQqIb
    0x000, // PshuflwVdqWdqIb
    0x000, // PinsrwPqEwIb
    0x000, // PextrwGdNqIb
    0x000, // ShufpsVpsWpsIb
    0x000, // PmovmskbGdNq
    0x000, // PminubPqQq
    0x000, // PmaxubPqQq
    0x000, // PavgbPqQq
    0x000, // PavgwPqQq
    0x000, // PmulhuwPqQq
    0x000, // MovntqMqPq
    0x000, // PminswPqQq
    0x000, // PmaxswPqQq
    0x000, // PsadbwPqQq
    0x000, // MaskmovqPqNq
    0x000, // AddpsVpsWps
    0x000, // AddpdVpdWpd
    0x000, // AddssVssWss
    0x000, // AddsdVsdWsd
    0x000, // MulpsVpsWps
    0x000, // MulpdVpdWpd
    0x000, // MulssVssWss
    0x000, // MulsdVsdWsd
    0x000, // SubpsVpsWps
    0x000, // SubpdVpdWpd
    0x000, // SubssVssWss
    0x000, // SubsdVsdWsd
    0x000, // MinpsVpsWps
    0x000, // MinpdVpdWpd
    0x000, // MinssVssWss
    0x000, // MinsdVsdWsd
    0x000, // DivpsVpsWps
    0x000, // DivpdVpdWpd
    0x000, // DivssVssWss
    0x000, // DivsdVsdWsd
    0x000, // MaxpsVpsWps
    0x000, // MaxpdVpdWpd
    0x000, // MaxssVssWss
    0x000, // MaxsdVsdWsd
    0x000, // SqrtpsVpsWps
    0x000, // SqrtpdVpdWpd
    0x000, // SqrtssVssWss
    0x000, // SqrtsdVsdWsd
    0x000, // CmppsVpsWpsIb
    0x000, // CmppdVpdWpdIb
    0x000, // CmpssVssWssIb
    0x000, // CmpsdVsdWsdIb
    0x000, // Cvtps2pdVpdWps
    0x000, // Cvtpd2psVpsWpd
    0x000, // Cvtss2sdVsdWss
    0x000, // Cvtsd2ssVssWsd
    0x000, // MovsdVsdWsd
    0x000, // MovsdWsdVsd
    0x000, // Cvtpi2pdVpdQq
    0x000, // Cvtsi2sdVsdEd
    0x000, // Cvttpd2piPqWpd
    0x000, // Cvttsd2siGdWsd
    0x000, // Cvtpd2piPqWpd
    0x000, // Cvtsd2siGdWsd
    0x000, // UcomisdVsdWsd
    0x000, // ComisdVsdWsd
    0x000, // Cvtdq2psVpsWdq
    0x000, // Cvtps2dqVdqWps
    0x000, // Cvttps2dqVdqWps
    0x000, // UnpckhpdVpdWdq
    0x000, // UnpcklpdVpdWdq
    0x000, // PunpckhdqVdqWdq
    0x000, // PunpckldqVdqWdq
    0x000, // MovapdVpdWpd
    0x000, // MovapdWpdVpd
    0x000, // MovdqaVdqWdq
    0x000, // MovdqaWdqVdq
    0x000, // MovdquVdqWdq
    0x000, // MovdquWdqVdq
    0x000, // MovhpdMqVsd
    0x000, // MovhpdVsdMq
    0x000, // MovlpdMqVsd
    0x000, // MovlpdVsdMq
    0x000, // MovntdqMdqVdq
    0x000, // MovntpdMpdVpd
    0x000, // MovupdVpdWpd
    0x000, // MovupdWpdVpd
    0x000, // AndnpdVpdWpd
    0x000, // AndpdVpdWpd
    0x000, // OrpdVpdWpd
    0x000, // XorpdVpdWpd
    0x000, // PandVdqWdq
    0x000, // PandnVdqWdq
    0x000, // PorVdqWdq
    0x000, // PxorVdqWdq
    0x000, // PunpcklbwVdqWdq
    0x000, // PunpcklwdVdqWdq
    0x000, // UnpcklpsVpsWdq
    0x000, // UnpckhpsVpsWdq
    0x000, // PackuswbVdqWdq
    0x000, // PacksswbVdqWdq
    0x000, // PcmpgtbVdqWdq
    0x000, // PcmpgtwVdqWdq
    0x000, // PcmpgtdVdqWdq
    0x000, // PunpckhbwVdqWdq
    0x000, // PunpckhwdVdqWdq
    0x000, // PackssdwVdqWdq
    0x000, // PunpcklqdqVdqWdq
    0x000, // PunpckhqdqVdqWdq
    0x000, // MovdVdqEd
    0x000, // PshufdVdqWdqIb
    0x000, // PshufhwVdqWdqIb
    0x000, // PcmpeqbVdqWdq
    0x000, // PcmpeqwVdqWdq
    0x000, // PcmpeqdVdqWdq
    0x000, // MovdEdVd
    0x000, // MovqVqWq
    0x000, // MovntiOp32MdGd
    0x000, // PinsrwVdqEwIb
    0x000, // PextrwGdUdqIb
    0x000, // ShufpdVpdWpdIb
    0x000, // PsrlwVdqWdq
    0x000, // PsrldVdqWdq
    0x000, // PsrlqVdqWdq
    0x000, // PaddqPqQq
    0x000, // PsubqPqQq
    0x000, // PaddqVdqWdq
    0x000, // PmullwVdqWdq
    0x000, // MovqWqVq
    0x000, // Movdq2qPqUdq
    0x000, // Movq2dqVdqQq
    0x000, // PmovmskbGdUdq
    0x000, // PsubusbVdqWdq
    0x000, // PsubuswVdqWdq
    0x000, // PminubVdqWdq
    0x000, // PaddusbVdqWdq
    0x000, // PadduswVdqWdq
    0x000, // PmaxubVdqWdq
    0x000, // PavgbVdqWdq
    0x000, // PsrawVdqWdq
    0x000, // PsradVdqWdq
    0x000, // PavgwVdqWdq
    0x000, // PmulhuwVdqWdq
    0x000, // PmulhwVdqWdq
    0x000, // Cvttpd2dqVqWpd
    0x000, // Cvtpd2dqVqWpd
    0x000, // Cvtdq2pdVpdWq
    0x000, // PsubsbVdqWdq
    0x000, // PsubswVdqWdq
    0x000, // PminswVdqWdq
    0x000, // PmaxswVdqWdq
    0x000, // PaddsbVdqWdq
    0x000, // PaddswVdqWdq
    0x000, // PsllwVdqWdq
    0x000, // PslldVdqWdq
    0x000, // PsllqVdqWdq
    0x000, // PmuludqPqQq
    0x000, // PmuludqVdqWdq
    0x000, // PmaddwdVdqWdq
    0x000, // PsadbwVdqWdq
    0x000, // MaskmovdquVdqUdq
    0x000, // PsubbVdqWdq
    0x000, // PsubwVdqWdq
    0x000, // PsubdVdqWdq
    0x000, // PsubqVdqWdq
    0x000, // PaddbVdqWdq
    0x000, // PaddwVdqWdq
    0x000, // PadddVdqWdq
    0x000, // PsrlwUdqIb
    0x000, // PsrawUdqIb
    0x000, // PsllwUdqIb
    0x000, // PsrldUdqIb
    0x000, // PsradUdqIb
    0x000, // PslldUdqIb
    0x000, // PsrlqUdqIb
    0x000, // PsllqUdqIb
    0x000, // PsrldqUdqIb
    0x000, // PslldqUdqIb
    0x000, // Lfence
    0x000, // Sfence
    0x000, // Mfence
    0x000, // MovddupVpdWq
    0x000, // MovsldupVpsWps
    0x000, // MovshdupVpsWps
    0x000, // HaddpdVpdWpd
    0x000, // HaddpsVpsWps
    0x000, // HsubpdVpdWpd
    0x000, // HsubpsVpsWps
    0x000, // AddsubpdVpdWpd
    0x000, // AddsubpsVpsWps
    0x000, // LddquVdqMdq
    0x000, // PshufbPqQq
    0x000, // PhaddwPqQq
    0x000, // PhadddPqQq
    0x000, // PhaddswPqQq
    0x000, // PmaddubswPqQq
    0x000, // PhsubswPqQq
    0x000, // PhsubwPqQq
    0x000, // PhsubdPqQq
    0x000, // PsignbPqQq
    0x000, // PsignwPqQq
    0x000, // PsigndPqQq
    0x000, // PmulhrswPqQq
    0x000, // PabsbPqQq
    0x000, // PabswPqQq
    0x000, // PabsdPqQq
    0x000, // PalignrPqQqIb
    0x000, // PshufbVdqWdq
    0x000, // PhaddwVdqWdq
    0x000, // PhadddVdqWdq
    0x000, // PhaddswVdqWdq
    0x000, // PmaddubswVdqWdq
    0x000, // PhsubswVdqWdq
    0x000, // PhsubwVdqWdq
    0x000, // PhsubdVdqWdq
    0x000, // PsignbVdqWdq
    0x000, // PsignwVdqWdq
    0x000, // PsigndVdqWdq
    0x000, // PmulhrswVdqWdq
    0x000, // PabsbVdqWdq
    0x000, // PabswVdqWdq
    0x000, // PabsdVdqWdq
    0x000, // PalignrVdqWdqIb
    0x000, // PblendvbVdqWdq
    0x000, // BlendvpsVpsWps
    0x000, // BlendvpdVpdWpd
    0x000, // PmovsxbwVdqWq
    0x000, // PmovsxbdVdqWd
    0x000, // PmovsxbqVdqWw
    0x000, // PmovsxwdVdqWq
    0x000, // PmovsxwqVdqWd
    0x000, // PmovsxdqVdqWq
    0x000, // PmovzxbwVdqWq
    0x000, // PmovzxbdVdqWd
    0x000, // PmovzxbqVdqWw
    0x000, // PmovzxwdVdqWq
    0x000, // PmovzxwqVdqWd
    0x000, // PmovzxdqVdqWq
    0x000, // PtestVdqWdq
    0x000, // PmuldqVdqWdq
    0x000, // PcmpeqqVdqWdq
    0x000, // PackusdwVdqWdq
    0x000, // PminsbVdqWdq
    0x000, // PminsdVdqWdq
    0x000, // PminuwVdqWdq
    0x000, // PminudVdqWdq
    0x000, // PmaxsbVdqWdq
    0x000, // PmaxsdVdqWdq
    0x000, // PmaxuwVdqWdq
    0x000, // PmaxudVdqWdq
    0x000, // PmulldVdqWdq
    0x000, // PhminposuwVdqWdq
    0x000, // RoundpsVpsWpsIb
    0x000, // RoundpdVpdWpdIb
    0x000, // RoundssVssWssIb
    0x000, // RoundsdVsdWsdIb
    0x000, // BlendpsVpsWpsIb
    0x000, // BlendpdVpdWpdIb
    0x000, // PblendwVdqWdqIb
    0x000, // PextrbEdVdqIbR
    0x000, // PextrbMbVdqIbM
    0x000, // PextrwEdVdqIbR
    0x000, // PextrwMwVdqIbM
    0x000, // PextrdEdVdqIb
    0x000, // PextrqEqVdqIb
    0x000, // ExtractpsEdVpsIb
    0x000, // PinsrbVdqEbIb
    0x000, // InsertpsVpsWssIb
    0x000, // PinsrdVdqEdIb
    0x000, // PinsrqVdqEqIb
    0x000, // DppsVpsWpsIb
    0x000, // DppdVpdWpdIb
    0x000, // MpsadbwVdqWdqIb
    0x000, // MovntdqaVdqMdq
    0x000, // Crc32GdEb
    0x000, // Crc32GdEw
    0x000, // Crc32GdEd
    0x000, // Crc32GdEq
    0x000, // PcmpgtqVdqWdq
    0x000, // PcmpestrmVdqWdqIb
    0x000, // PcmpestriVdqWdqIb
    0x000, // PcmpistrmVdqWdqIb
    0x000, // PcmpistriVdqWdqIb
    0x000, // MovbeGwMw
    0x000, // MovbeGdMd
    0x000, // MovbeGqMq
    0x000, // MovbeMwGw
    0x000, // MovbeMdGd
    0x000, // MovbeMqGq
    0x000, // PopcntGwEw
    0x000, // PopcntGdEd
    0x000, // PopcntGqEq
    0x000, // Xrstor
    0x000, // Xsave
    0x000, // Xsavec
    0x000, // Xsetbv
    0x000, // Xgetbv
    0x000, // Xsaveopt
    0x000, // Xsaves
    0x000, // Xrstors
    0x000, // AesimcVdqWdq
    0x000, // AeskeygenassistVdqWdqIb
    0x000, // AesencVdqWdq
    0x000, // AesenclastVdqWdq
    0x000, // AesdecVdqWdq
    0x000, // AesdeclastVdqWdq
    0x000, // PclmulqdqVdqWdqIb
    0x000, // Sha1nexteVdqWdq
    0x000, // Sha1msg1VdqWdq
    0x000, // Sha1msg2VdqWdq
    0x000, // Sha256rnds2VdqWdq
    0x000, // Sha256msg1VdqWdq
    0x000, // Sha256msg2VdqWdq
    0x000, // Sha1rnds4VdqWdqIb
    0x000, // Gf2p8affineqbVdqWdqIb
    0x000, // Gf2p8affineinvqbVdqWdqIb
    0x000, // Gf2p8mulbVdqWdq
    0x000, // LahfLm
    0x000, // SahfLm
    0x000, // Syscall
    0x000, // Sysret
    0x000, // XorEqGqZeroIdiom
    0x000, // XorGqEqZeroIdiom
    0x000, // SubEqGqZeroIdiom
    0x000, // SubGqEqZeroIdiom
    0x000, // AddGqEq
    0x000, // OrGqEq
    0x000, // AdcGqEq
    0x000, // SbbGqEq
    0x000, // AndGqEq
    0x000, // SubGqEq
    0x000, // XorGqEq
    0x000, // CmpGqEq
    0x000, // AddEqGq
    0x000, // OrEqGq
    0x000, // AdcEqGq
    0x000, // SbbEqGq
    0x000, // AndEqGq
    0x000, // SubEqGq
    0x000, // XorEqGq
    0x000, // TestEqGq
    0x000, // CmpEqGq
    0x000, // AddRaxid
    0x000, // OrRaxid
    0x000, // AdcRaxid
    0x000, // SbbRaxid
    0x000, // AndRaxid
    0x000, // SubRaxid
    0x000, // XorRaxid
    0x000, // TestRaxid
    0x000, // CmpRaxid
    0x000, // AddEqId
    0x000, // OrEqId
    0x000, // AdcEqId
    0x000, // SbbEqId
    0x000, // AndEqId
    0x000, // SubEqId
    0x000, // XorEqId
    0x000, // TestEqId
    0x000, // CmpEqId
    0x000, // AddEqsIb
    0x000, // OrEqsIb
    0x000, // AdcEqsIb
    0x000, // SbbEqsIb
    0x000, // AndEqsIb
    0x000, // SubEqsIb
    0x000, // XorEqsIb
    0x000, // TestEqsIb
    0x000, // CmpEqsIb
    0x000, // XchgEqGq
    0x000, // XchgRrxRax
    0x000, // LeaGqM
    0x000, // MovOp64GdEd
    0x000, // MovOp64EdGd
    0x000, // MovGqEq
    0x000, // MovEqGq
    0x000, // MovEqId
    0x000, // MovRaxoq
    0x000, // MovOqRax
    0x000, // MovEaxoq
    0x000, // MovOqEax
    0x000, // MovAxoq
    0x000, // MovOqAx
    0x000, // MovAloq
    0x000, // MovOqAl
    0x000, // RepMovsqYqXq
    0x000, // RepCmpsqXqYq
    0x000, // RepStosqYqRax
    0x000, // RepLodsqRaxxq
    0x000, // RepScasqRaxyq
    0x000, // CallJq
    0x000, // JmpJq
    0x000, // JmpJbq
    0x000, // JoJq
    0x000, // JnoJq
    0x000, // JbJq
    0x000, // JnbJq
    0x000, // JzJq
    0x000, // JnzJq
    0x000, // JbeJq
    0x000, // JnbeJq
    0x000, // JsJq
    0x000, // JnsJq
    0x000, // JpJq
    0x000, // JnpJq
    0x000, // JlJq
    0x000, // JnlJq
    0x000, // JleJq
    0x000, // JnleJq
    0x000, // JoJbq
    0x000, // JnoJbq
    0x000, // JbJbq
    0x000, // JnbJbq
    0x000, // JzJbq
    0x000, // JnzJbq
    0x000, // JbeJbq
    0x000, // JnbeJbq
    0x000, // JsJbq
    0x000, // JnsJbq
    0x000, // JpJbq
    0x000, // JnpJbq
    0x000, // JlJbq
    0x000, // JnlJbq
    0x000, // JleJbq
    0x000, // JnleJbq
    0x000, // EnterOp64IwIb
    0x000, // LeaveOp64
    0x000, // IretOp64
    0x000, // ShldEqGq
    0x000, // ShldEqGqIb
    0x000, // ShrdEqGq
    0x000, // ShrdEqGqIb
    0x000, // ImulGqEq
    0x000, // ImulGqEqId
    0x000, // ImulGqEqsIb
    0x000, // MovzxGqEb
    0x000, // MovzxGqEw
    0x000, // MovsxGqEb
    0x000, // MovsxGqEw
    0x000, // MovsxdGqEd
    0x000, // BswapRrx
    0x000, // BsfGqEq
    0x000, // BsrGqEq
    0x000, // BtEqGq
    0x000, // BtsEqGq
    0x000, // BtrEqGq
    0x000, // BtcEqGq
    0x000, // BtEqIb
    0x000, // BtsEqIb
    0x000, // BtrEqIb
    0x000, // BtcEqIb
    0x000, // NotEq
    0x000, // NegEq
    0x000, // RolEq
    0x000, // RorEq
    0x000, // RclEq
    0x000, // RcrEq
    0x000, // ShlEq
    0x000, // ShrEq
    0x000, // SarEq
    0x000, // RolEqIb
    0x000, // RorEqIb
    0x000, // RclEqIb
    0x000, // RcrEqIb
    0x000, // ShlEqIb
    0x000, // ShrEqIb
    0x000, // SarEqIb
    0x000, // RolEqI1
    0x000, // RorEqI1
    0x000, // RclEqI1
    0x000, // RcrEqI1
    0x000, // ShlEqI1
    0x000, // ShrEqI1
    0x000, // SarEqI1
    0x000, // MulRaxeq
    0x000, // ImulRaxeq
    0x000, // DivRaxeq
    0x000, // IdivRaxeq
    0x000, // IncEq
    0x000, // DecEq
    0x000, // CallEq
    0x000, // CallfOp64Ep
    0x000, // JmpEq
    0x000, // JmpfOp64Ep
    0x000, // PushfFq
    0x000, // PopfFq
    0x000, // CmpxchgEqGq
    0x000, // Cdqe
    0x000, // Cqo
    0x000, // XaddEqGq
    0x000, // RetOp64Iw
    0x000, // RetOp64
    0x000, // RetfOp64Iw
    0x000, // RetfOp64
    0x000, // CmovoGqEq
    0x000, // CmovnoGqEq
    0x000, // CmovbGqEq
    0x000, // CmovnbGqEq
    0x000, // CmovzGqEq
    0x000, // CmovnzGqEq
    0x000, // CmovbeGqEq
    0x000, // CmovnbeGqEq
    0x000, // CmovsGqEq
    0x000, // CmovnsGqEq
    0x000, // CmovpGqEq
    0x000, // CmovnpGqEq
    0x000, // CmovlGqEq
    0x000, // CmovnlGqEq
    0x000, // CmovleGqEq
    0x000, // CmovnleGqEq
    0x000, // PushEq
    0x000, // PopEq
    0x000, // PushOp64Id
    0x000, // PushOp64SIb
    0x000, // PushOp64Sw
    0x000, // PopOp64Sw
    0x000, // SgdtOp64Ms
    0x000, // SidtOp64Ms
    0x000, // LgdtOp64Ms
    0x000, // LidtOp64Ms
    0x000, // MovRrxiq
    0x000, // LssGqMp
    0x000, // LfsGqMp
    0x000, // LgsGqMp
    0x000, // CMPXCHG16B
    0x000, // LoopneJbq
    0x000, // LoopeJbq
    0x000, // LoopJbq
    0x000, // JrcxzJbq
    0x000, // MovqEqVq
    0x000, // MovqPqEq
    0x000, // MovqVdqEq
    0x000, // Cvtsi2ssVssEq
    0x000, // Cvtsi2sdVsdEq
    0x000, // Cvttss2siGqWss
    0x000, // Cvttsd2siGqWsd
    0x000, // Cvtss2siGqWss
    0x000, // Cvtsd2siGqWsd
    0x000, // MovntiOp64MdGd
    0x000, // MovntiMqGq
    0x000, // MovCr0rq
    0x000, // MovCr2rq
    0x000, // MovCr3rq
    0x000, // MovCr4rq
    0x000, // MovRqCr0
    0x000, // MovRqCr2
    0x000, // MovRqCr3
    0x000, // MovRqCr4
    0x000, // MovDqRq
    0x000, // MovRqDq
    0x000, // Swapgs
    0x000, // RdfsbaseEd
    0x000, // RdgsbaseEd
    0x000, // RdfsbaseEq
    0x000, // RdgsbaseEq
    0x000, // WrfsbaseEd
    0x000, // WrgsbaseEd
    0x000, // WrfsbaseEq
    0x000, // WrgsbaseEq
    0x000, // Rdtscp
    0x000, // VmxonMq
    0x000, // Vmxoff
    0x000, // Vmcall
    0x000, // Vmlaunch
    0x000, // Vmresume
    0x000, // VmclearMq
    0x000, // VmptrldMq
    0x000, // VmptrstMq
    0x000, // VmreadEdGd
    0x000, // VmwriteGdEd
    0x000, // VmreadEqGq
    0x000, // VmwriteGqEq
    0x000, // Invept
    0x000, // Invvpid
    0x000, // Vmfunc
    0x000, // Getsec
    0x000, // Vmrun
    0x000, // Vmmcall
    0x000, // Vmload
    0x000, // Vmsave
    0x000, // Stgi
    0x000, // Clgi
    0x000, // Skinit
    0x000, // Invlpga
    0x000, // Incsspd
    0x000, // Incsspq
    0x000, // Rdsspd
    0x000, // Rdsspq
    0x000, // Saveprevssp
    0x000, // Rstorssp
    0x000, // Wrssd
    0x000, // Wrussd
    0x000, // Wrssq
    0x000, // Wrussq
    0x000, // Setssbsy
    0x000, // Clrssbsy
    0x000, // Endbranch32
    0x000, // Endbranch64
    0x000, // Invpcid
    0x000, // Rdpkru
    0x000, // Wrpkru
    0x000, // Clui
    0x000, // Stui
    0x000, // Testui
    0x000, // Uiret
    0x000, // SenduipiEq
    0x000, // RdpidEd
    0x000, // Serialize
    0x000, // Wrmsrns
    0x000, // Rdmsrlist
    0x000, // Wrmsrlist
    0x000, // Vzeroupper
    0x000, // Vzeroall
    0x000, // Vldmxcsr
    0x000, // Vstmxcsr
    0x000, // VmovapsVpsWps
    0x000, // V128VmovapsWpsVps
    0x000, // V256VmovapsWpsVps
    0x000, // VmovapdVpdWpd
    0x000, // V128VmovapdWpdVpd
    0x000, // V256VmovapdWpdVpd
    0x000, // VmovupsVpsWps
    0x000, // V128VmovupsWpsVps
    0x000, // V256VmovupsWpsVps
    0x000, // VmovupdVpdWpd
    0x000, // V128VmovupdWpdVpd
    0x000, // V256VmovupdWpdVpd
    0x000, // VmovdqaVdqWdq
    0x000, // V128VmovdqaWdqVdq
    0x000, // V256VmovdqaWdqVdq
    0x000, // VmovdquVdqWdq
    0x000, // V128VmovdquWdqVdq
    0x000, // V256VmovdquWdqVdq
    0x000, // V128VmovsdVsdHpdWsd
    0x000, // V128VmovssVssHpsWss
    0x000, // V128VmovsdWsdHpdVsd
    0x000, // V128VmovssWssHpsVss
    0x000, // V128VmovsdVsdWsd
    0x000, // V128VmovssVssWss
    0x000, // V128VmovsdWsdVsd
    0x000, // V128VmovssWssVss
    0x000, // V128VmovlpsVpsHpsMq
    0x000, // V128VmovhlpsVpsHpsWps
    0x000, // V128VmovhpsVpsHpsMq
    0x000, // V128VmovlhpsVpsHpsWps
    0x000, // V128VmovlpsMqVps
    0x000, // V128VmovhpsMqVps
    0x000, // V128VmovlpdMqVsd
    0x000, // V128VmovhpdMqVsd
    0x000, // V128VmovlpdVpdHpdMq
    0x000, // V128VmovhpdVpdHpdMq
    0x000, // V128VmovddupVpdWpd
    0x000, // V256VmovddupVpdWpd
    0x000, // VmovsldupVpsWps
    0x000, // VmovshdupVpsWps
    0x000, // VlddquVdqMdq
    0x000, // V128VmovntdqaVdqMdq
    0x000, // V256VmovntdqaVdqMdq
    0x000, // V128VmovntpsMpsVps
    0x000, // V256VmovntpsMpsVps
    0x000, // V128VmovntpdMpdVpd
    0x000, // V256VmovntpdMpdVpd
    0x000, // V128VmovntdqMdqVdq
    0x000, // V256VmovntdqMdqVdq
    0x000, // VucomissVssWss
    0x000, // VcomissVssWss
    0x000, // VucomisdVsdWsd
    0x000, // VcomisdVsdWsd
    0x000, // VrsqrtssVssHpsWss
    0x000, // VrsqrtpsVpsWps
    0x000, // VrcpssVssHpsWss
    0x000, // VrcppsVpsWps
    0x000, // VandpsVpsHpsWps
    0x000, // VandpdVpdHpdWpd
    0x000, // VandnpsVpsHpsWps
    0x000, // VandnpdVpdHpdWpd
    0x000, // VorpsVpsHpsWps
    0x000, // VorpdVpdHpdWpd
    0x000, // VxorpsVpsHpsWps
    0x000, // VxorpdVpdHpdWpd
    0x000, // V128VpshufdVdqWdqIb
    0x000, // V256VpshufdVdqWdqIb
    0x000, // V128VpshufhwVdqWdqIb
    0x000, // V256VpshufhwVdqWdqIb
    0x000, // V128VpshuflwVdqWdqIb
    0x000, // V256VpshuflwVdqWdqIb
    0x000, // VhaddpdVpdHpdWpd
    0x000, // VhaddpsVpsHpsWps
    0x000, // VhsubpdVpdHpdWpd
    0x000, // VhsubpsVpsHpsWps
    0x000, // VshufpsVpsHpsWpsIb
    0x000, // VshufpdVpdHpdWpdIb
    0x000, // VaddsubpdVpdHpdWpd
    0x000, // VaddsubpsVpsHpsWps
    0x000, // VroundpsVpsWpsIb
    0x000, // VroundpdVpdWpdIb
    0x000, // VroundsdVsdHpdWsdIb
    0x000, // VroundssVssHpsWssIb
    0x000, // VdppsVpsHpsWpsIb
    0x000, // VdppdVpdHpdWpdIb
    0x000, // VaddpsVpsHpsWps
    0x000, // VaddpdVpdHpdWpd
    0x000, // VaddssVssHpsWss
    0x000, // VaddsdVsdHpdWsd
    0x000, // VmulpsVpsHpsWps
    0x000, // VmulpdVpdHpdWpd
    0x000, // VmulssVssHpsWss
    0x000, // VmulsdVsdHpdWsd
    0x000, // VsubpsVpsHpsWps
    0x000, // VsubpdVpdHpdWpd
    0x000, // VsubssVssHpsWss
    0x000, // VsubsdVsdHpdWsd
    0x000, // VdivpsVpsHpsWps
    0x000, // VdivpdVpdHpdWpd
    0x000, // VdivssVssHpsWss
    0x000, // VdivsdVsdHpdWsd
    0x000, // VmaxpsVpsHpsWps
    0x000, // VmaxpdVpdHpdWpd
    0x000, // VmaxssVssHpsWss
    0x000, // VmaxsdVsdHpdWsd
    0x000, // VminpsVpsHpsWps
    0x000, // VminpdVpdHpdWpd
    0x000, // VminssVssHpsWss
    0x000, // VminsdVsdHpdWsd
    0x000, // VsqrtpsVpsWps
    0x000, // VsqrtpdVpdWpd
    0x000, // VsqrtssVssHpsWss
    0x000, // VsqrtsdVsdHpdWsd
    0x000, // VcmppsVpsHpsWpsIb
    0x000, // VcmppdVpdHpdWpdIb
    0x000, // VcmpssVssHpsWssIb
    0x000, // VcmpsdVsdHpdWsdIb
    0x000, // V128VpsrlwVdqHdqWdq
    0x000, // V256VpsrlwVdqHdqWdq
    0x000, // V128VpsrldVdqHdqWdq
    0x000, // V256VpsrldVdqHdqWdq
    0x000, // V128VpsrlqVdqHdqWdq
    0x000, // V256VpsrlqVdqHdqWdq
    0x000, // V128VpsrawVdqHdqWdq
    0x000, // V256VpsrawVdqHdqWdq
    0x000, // V128VpsradVdqHdqWdq
    0x000, // V256VpsradVdqHdqWdq
    0x000, // V128VpsllwVdqHdqWdq
    0x000, // V256VpsllwVdqHdqWdq
    0x000, // V128VpslldVdqHdqWdq
    0x000, // V256VpslldVdqHdqWdq
    0x000, // V128VpsllqVdqHdqWdq
    0x000, // V256VpsllqVdqHdqWdq
    0x000, // V128VpsrlwUdqIb
    0x000, // V256VpsrlwUdqIb
    0x000, // V128VpsrawUdqIb
    0x000, // V256VpsrawUdqIb
    0x000, // V128VpsllwUdqIb
    0x000, // V256VpsllwUdqIb
    0x000, // V128VpsrldUdqIb
    0x000, // V256VpsrldUdqIb
    0x000, // V128VpsradUdqIb
    0x000, // V256VpsradUdqIb
    0x000, // V128VpslldUdqIb
    0x000, // V256VpslldUdqIb
    0x000, // V128VpsrlqUdqIb
    0x000, // V256VpsrlqUdqIb
    0x000, // V128VpsllqUdqIb
    0x000, // V256VpsllqUdqIb
    0x000, // V128VpsrldqUdqIb
    0x000, // V256VpsrldqUdqIb
    0x000, // V128VpslldqUdqIb
    0x000, // V256VpslldqUdqIb
    0x000, // V128VpmovmskbGdUdq
    0x000, // V256VpmovmskbGdUdq
    0x000, // VmovmskpsGdUps
    0x000, // VmovmskpdGdUpd
    0x000, // VunpcklpdVpdHpdWpd
    0x000, // VunpckhpdVpdHpdWpd
    0x000, // VunpcklpsVpsHpsWps
    0x000, // VunpckhpsVpsHpsWps
    0x000, // V128VpunpckhdqVdqHdqWdq
    0x000, // V256VpunpckhdqVdqHdqWdq
    0x000, // V128VpunpckldqVdqHdqWdq
    0x000, // V256VpunpckldqVdqHdqWdq
    0x000, // V128VpunpcklbwVdqHdqWdq
    0x000, // V256VpunpcklbwVdqHdqWdq
    0x000, // V128VpunpcklwdVdqHdqWdq
    0x000, // V256VpunpcklwdVdqHdqWdq
    0x000, // V128VpunpckhbwVdqHdqWdq
    0x000, // V256VpunpckhbwVdqHdqWdq
    0x000, // V128VpunpckhwdVdqHdqWdq
    0x000, // V256VpunpckhwdVdqHdqWdq
    0x000, // V128VpunpcklqdqVdqHdqWdq
    0x000, // V256VpunpcklqdqVdqHdqWdq
    0x000, // V128VpunpckhqdqVdqHdqWdq
    0x000, // V256VpunpckhqdqVdqHdqWdq
    0x000, // V128VpcmpeqbVdqHdqWdq
    0x000, // V256VpcmpeqbVdqHdqWdq
    0x000, // V128VpcmpeqwVdqHdqWdq
    0x000, // V256VpcmpeqwVdqHdqWdq
    0x000, // V128VpcmpeqdVdqHdqWdq
    0x000, // V256VpcmpeqdVdqHdqWdq
    0x000, // V128VpcmpeqqVdqHdqWdq
    0x000, // V256VpcmpeqqVdqHdqWdq
    0x000, // V128VpcmpgtbVdqHdqWdq
    0x000, // V256VpcmpgtbVdqHdqWdq
    0x000, // V128VpcmpgtwVdqHdqWdq
    0x000, // V256VpcmpgtwVdqHdqWdq
    0x000, // V128VpcmpgtdVdqHdqWdq
    0x000, // V256VpcmpgtdVdqHdqWdq
    0x000, // V128VpcmpgtqVdqHdqWdq
    0x000, // V256VpcmpgtqVdqHdqWdq
    0x000, // V128VpsubsbVdqHdqWdq
    0x000, // V256VpsubsbVdqHdqWdq
    0x000, // V128VpsubswVdqHdqWdq
    0x000, // V256VpsubswVdqHdqWdq
    0x000, // V128VpaddsbVdqHdqWdq
    0x000, // V256VpaddsbVdqHdqWdq
    0x000, // V128VpaddswVdqHdqWdq
    0x000, // V256VpaddswVdqHdqWdq
    0x000, // V128VpsubusbVdqHdqWdq
    0x000, // V256VpsubusbVdqHdqWdq
    0x000, // V128VpsubuswVdqHdqWdq
    0x000, // V256VpsubuswVdqHdqWdq
    0x000, // V128VpaddusbVdqHdqWdq
    0x000, // V256VpaddusbVdqHdqWdq
    0x000, // V128VpadduswVdqHdqWdq
    0x000, // V256VpadduswVdqHdqWdq
    0x000, // V128VpavgbVdqWdq
    0x000, // V256VpavgbVdqWdq
    0x000, // V128VpavgwVdqWdq
    0x000, // V256VpavgwVdqWdq
    0x000, // V128VpandnVdqHdqWdq
    0x000, // V256VpandnVdqHdqWdq
    0x000, // V128VpandVdqHdqWdq
    0x000, // V256VpandVdqHdqWdq
    0x000, // V128VporVdqHdqWdq
    0x000, // V256VporVdqHdqWdq
    0x000, // V128VpxorVdqHdqWdq
    0x000, // V256VpxorVdqHdqWdq
    0x000, // V128VpmulhrswVdqHdqWdq
    0x000, // V256VpmulhrswVdqHdqWdq
    0x000, // V128VpmuldqVdqHdqWdq
    0x000, // V256VpmuldqVdqHdqWdq
    0x000, // V128VpmuludqVdqHdqWdq
    0x000, // V256VpmuludqVdqHdqWdq
    0x000, // V128VpmulldVdqHdqWdq
    0x000, // V256VpmulldVdqHdqWdq
    0x000, // V128VpmullwVdqHdqWdq
    0x000, // V256VpmullwVdqHdqWdq
    0x000, // V128VpmulhwVdqHdqWdq
    0x000, // V256VpmulhwVdqHdqWdq
    0x000, // V128VpmulhuwVdqHdqWdq
    0x000, // V256VpmulhuwVdqHdqWdq
    0x000, // V128VpsadbwVdqHdqWdq
    0x000, // V256VpsadbwVdqHdqWdq
    0x000, // V128VmaskmovdquVdqUdq
    0x000, // V128VpsubbVdqHdqWdq
    0x000, // V256VpsubbVdqHdqWdq
    0x000, // V128VpsubwVdqHdqWdq
    0x000, // V256VpsubwVdqHdqWdq
    0x000, // V128VpsubdVdqHdqWdq
    0x000, // V256VpsubdVdqHdqWdq
    0x000, // V128VpsubqVdqHdqWdq
    0x000, // V256VpsubqVdqHdqWdq
    0x000, // V128VpaddbVdqHdqWdq
    0x000, // V256VpaddbVdqHdqWdq
    0x000, // V128VpaddwVdqHdqWdq
    0x000, // V256VpaddwVdqHdqWdq
    0x000, // V128VpadddVdqHdqWdq
    0x000, // V256VpadddVdqHdqWdq
    0x000, // V128VpaddqVdqHdqWdq
    0x000, // V256VpaddqVdqHdqWdq
    0x000, // V128VpshufbVdqHdqWdq
    0x000, // V256VpshufbVdqHdqWdq
    0x000, // V128VphaddwVdqHdqWdq
    0x000, // V256VphaddwVdqHdqWdq
    0x000, // V128VphadddVdqHdqWdq
    0x000, // V256VphadddVdqHdqWdq
    0x000, // V128VphsubwVdqHdqWdq
    0x000, // V256VphsubwVdqHdqWdq
    0x000, // V128VphsubdVdqHdqWdq
    0x000, // V256VphsubdVdqHdqWdq
    0x000, // V128VphaddswVdqHdqWdq
    0x000, // V256VphaddswVdqHdqWdq
    0x000, // V128VphsubswVdqHdqWdq
    0x000, // V256VphsubswVdqHdqWdq
    0x000, // V128VpmaddwdVdqHdqWdq
    0x000, // V256VpmaddwdVdqHdqWdq
    0x000, // V128VpmaddubswVdqHdqWdq
    0x000, // V256VpmaddubswVdqHdqWdq
    0x000, // V128VpsignbVdqHdqWdq
    0x000, // V256VpsignbVdqHdqWdq
    0x000, // V128VpsignwVdqHdqWdq
    0x000, // V256VpsignwVdqHdqWdq
    0x000, // V128VpsigndVdqHdqWdq
    0x000, // V256VpsigndVdqHdqWdq
    0x000, // VtestpsVpsWps
    0x000, // VtestpdVpdWpd
    0x000, // VptestVdqWdq
    0x000, // VbroadcastssVpsMss
    0x000, // V256VbroadcastsdVpdMsd
    0x000, // V256Vbroadcastf128VdqMdq
    0x000, // V128VpabsbVdqWdq
    0x000, // V256VpabsbVdqWdq
    0x000, // V128VpabswVdqWdq
    0x000, // V256VpabswVdqWdq
    0x000, // V128VpabsdVdqWdq
    0x000, // V256VpabsdVdqWdq
    0x000, // V128VpacksswbVdqHdqWdq
    0x000, // V256VpacksswbVdqHdqWdq
    0x000, // V128VpackuswbVdqHdqWdq
    0x000, // V256VpackuswbVdqHdqWdq
    0x000, // V128VpackusdwVdqHdqWdq
    0x000, // V256VpackusdwVdqHdqWdq
    0x000, // V128VpackssdwVdqHdqWdq
    0x000, // V256VpackssdwVdqHdqWdq
    0x000, // VmaskmovpsVpsHpsMps
    0x000, // VmaskmovpdVpdHpdMpd
    0x000, // VmaskmovpsMpsHpsVps
    0x000, // VmaskmovpdMpdHpdVpd
    0x000, // V128VpmovsxbwVdqWq
    0x000, // V128VpmovsxbdVdqWd
    0x000, // V128VpmovsxbqVdqWw
    0x000, // V128VpmovsxwdVdqWq
    0x000, // V128VpmovsxwqVdqWd
    0x000, // V128VpmovsxdqVdqWq
    0x000, // V128VpmovzxbwVdqWq
    0x000, // V128VpmovzxbdVdqWd
    0x000, // V128VpmovzxbqVdqWw
    0x000, // V128VpmovzxwdVdqWq
    0x000, // V128VpmovzxwqVdqWd
    0x000, // V128VpmovzxdqVdqWq
    0x000, // V128VpminsbVdqHdqWdq
    0x000, // V256VpminsbVdqHdqWdq
    0x000, // V128VpminswVdqHdqWdq
    0x000, // V256VpminswVdqHdqWdq
    0x000, // V128VpminsdVdqHdqWdq
    0x000, // V256VpminsdVdqHdqWdq
    0x000, // V128VpminubVdqHdqWdq
    0x000, // V256VpminubVdqHdqWdq
    0x000, // V128VpminuwVdqHdqWdq
    0x000, // V256VpminuwVdqHdqWdq
    0x000, // V128VpminudVdqHdqWdq
    0x000, // V256VpminudVdqHdqWdq
    0x000, // V128VpmaxsbVdqHdqWdq
    0x000, // V256VpmaxsbVdqHdqWdq
    0x000, // V128VpmaxswVdqHdqWdq
    0x000, // V256VpmaxswVdqHdqWdq
    0x000, // V128VpmaxsdVdqHdqWdq
    0x000, // V256VpmaxsdVdqHdqWdq
    0x000, // V128VpmaxubVdqHdqWdq
    0x000, // V256VpmaxubVdqHdqWdq
    0x000, // V128VpmaxuwVdqHdqWdq
    0x000, // V256VpmaxuwVdqHdqWdq
    0x000, // V128VpmaxudVdqHdqWdq
    0x000, // V256VpmaxudVdqHdqWdq
    0x000, // V128VphminposuwVdqWdq
    0x000, // VpermilpsVpsHpsWps
    0x000, // VpermilpdVpdHpdWpd
    0x000, // VpermilpsVpsWpsIb
    0x000, // VpermilpdVpdWpdIb
    0x000, // VblendpsVpsHpsWpsIb
    0x000, // VblendpdVpdHpdWpdIb
    0x000, // V128VpblendwVdqHdqWdqIb
    0x000, // V256VpblendwVdqHdqWdqIb
    0x000, // V128VpalignrVdqHdqWdqIb
    0x000, // V256VpalignrVdqHdqWdqIb
    0x000, // V128VinsertpsVpsWssIb
    0x000, // V128VextractpsEdVpsIb
    0x000, // V256Vperm2f128VdqHdqWdqIb
    0x000, // V256Vinsertf128VdqHdqWdqIb
    0x000, // V256Vextractf128WdqVdqIb
    0x000, // VblendvpsVpsHpsWpsIb
    0x000, // VblendvpdVpdHpdWpdIb
    0x000, // V128VpblendvbVdqHdqWdqIb
    0x000, // V256VpblendvbVdqHdqWdqIb
    0x000, // V128VmpsadbwVdqHdqWdqIb
    0x000, // V256VmpsadbwVdqHdqWdqIb
    0x000, // V128VpcmpestrmVdqWdqIb
    0x000, // V128VpcmpestriVdqWdqIb
    0x000, // V128VpcmpistrmVdqWdqIb
    0x000, // V128VpcmpistriVdqWdqIb
    0x000, // V128VaesimcVdqWdq
    0x000, // V128VaeskeygenassistVdqWdqIb
    0x000, // V128VaesencVdqHdqWdq
    0x000, // V128VaesenclastVdqHdqWdq
    0x000, // V128VaesdecVdqHdqWdq
    0x000, // V128VaesdeclastVdqHdqWdq
    0x000, // V128VpclmulqdqVdqHdqWdqIb
    0x000, // V256VaesencVdqHdqWdq
    0x000, // V256VaesenclastVdqHdqWdq
    0x000, // V256VaesdecVdqHdqWdq
    0x000, // V256VaesdeclastVdqHdqWdq
    0x000, // V256VpclmulqdqVdqHdqWdqIb
    0x000, // Vgf2p8affineqbVdqHdqWdqIb
    0x000, // Vgf2p8affineinvqbVdqHdqWdqIb
    0x000, // Vgf2p8mulbVdqHdqWdq
    0x000, // Vsm3msg1VdqHdqWdq
    0x000, // Vsm3msg2VdqHdqWdq
    0x000, // Vsm3rnds2VdqHdqWdqIb
    0x000, // Vsm4key4VdqHdqWdq
    0x000, // Vsm4rnds4VdqHdqWdq
    0x000, // Vsha512msg1VdqWdq
    0x000, // Vsha512msg2VdqWdq
    0x000, // Vsha512rnds2VdqHdqWdq
    0x000, // V128VmovdVdqEd
    0x000, // V128VmovqVdqEq
    0x000, // V128VmovdEdVd
    0x000, // V128VmovqEqVq
    0x000, // V128VpinsrbVdqEbIb
    0x000, // V128VpinsrwVdqEwIb
    0x000, // V128VpextrwGdUdqIb
    0x000, // V128VpextrbEdVdqIbR
    0x000, // V128VpextrbMbVdqIbM
    0x000, // V128VpextrwEdVdqIbR
    0x000, // V128VpextrwMwVdqIbM
    0x000, // V128VpinsrdVdqEdIb
    0x000, // V128VpinsrqVdqEqIb
    0x000, // V128VpextrdEdVdqIb
    0x000, // V128VpextrqEqVdqIb
    0x000, // Vcvtps2pdVpdWps
    0x000, // Vcvttpd2dqVdqWpd
    0x000, // Vcvtpd2dqVdqWpd
    0x000, // Vcvtdq2pdVpdWdq
    0x000, // Vcvtpd2psVpsWpd
    0x000, // Vcvtsd2ssVssWsd
    0x000, // Vcvtss2sdVsdWss
    0x000, // Vcvtdq2psVpsWdq
    0x000, // Vcvtps2dqVdqWps
    0x000, // Vcvttps2dqVdqWps
    0x000, // Vcvtss2siGdWss
    0x000, // Vcvtss2siGqWss
    0x000, // Vcvtsd2siGdWsd
    0x000, // Vcvtsd2siGqWsd
    0x000, // Vcvttss2siGdWss
    0x000, // Vcvttss2siGqWss
    0x000, // Vcvttsd2siGdWsd
    0x000, // Vcvttsd2siGqWsd
    0x000, // Vcvtsi2ssVssEd
    0x000, // Vcvtsi2ssVssEq
    0x000, // Vcvtsi2sdVsdEd
    0x000, // Vcvtsi2sdVsdEq
    0x000, // VmovqWqVq
    0x000, // VmovqVqWq
    0x000, // Vcvtph2psVpsWps
    0x000, // Vcvtps2phWpsVpsIb
    0x000, // V256VpmovsxbwVdqWdq
    0x000, // V256VpmovsxbdVdqWq
    0x000, // V256VpmovsxbqVdqWd
    0x000, // V256VpmovsxwdVdqWdq
    0x000, // V256VpmovsxwqVdqWq
    0x000, // V256VpmovsxdqVdqWdq
    0x000, // V256VpmovzxbwVdqWdq
    0x000, // V256VpmovzxbdVdqWq
    0x000, // V256VpmovzxbqVdqWd
    0x000, // V256VpmovzxwdVdqWdq
    0x000, // V256VpmovzxwqVdqWq
    0x000, // V256VpmovzxdqVdqWdq
    0x000, // V256Vperm2i128VdqHdqWdqIb
    0x000, // V256Vinserti128VdqHdqWdqIb
    0x000, // V256Vextracti128WdqVdqIb
    0x000, // V256Vbroadcasti128VdqMdq
    0x000, // VpbroadcastbVdqWb
    0x000, // VpbroadcastwVdqWw
    0x000, // VpbroadcastdVdqWd
    0x000, // VpbroadcastqVdqWq
    0x000, // VbroadcastssVpsWss
    0x000, // V256VbroadcastsdVpdWsd
    0x000, // VpblenddVdqHdqWdqIb
    0x000, // VmaskmovdVdqHdqMdq
    0x000, // VmaskmovqVdqHdqMdq
    0x000, // VmaskmovdMdqHdqVdq
    0x000, // VmaskmovqMdqHdqVdq
    0x000, // VgatherdpsVpsHps
    0x000, // VgatherdpdVpdHpd
    0x000, // VgatherqpsVpsHps
    0x000, // VgatherqpdVpdHpd
    0x000, // VgatherddVdqHdq
    0x000, // VgatherdqVdqHdq
    0x000, // VgatherqdVdqHdq
    0x000, // VgatherqqVdqHdq
    0x000, // VpsrlvdVdqHdqWdq
    0x000, // VpsrlvqVdqHdqWdq
    0x000, // VpsllvdVdqHdqWdq
    0x000, // VpsllvqVdqHdqWdq
    0x000, // V256VpermqVdqWdqIb
    0x000, // V256VpermdVdqHdqWdq
    0x000, // V256VpermpsVpsHpsWps
    0x000, // V256VpermpdVpdWpdIb
    0x000, // VpsravdVdqHdqWdq
    0x000, // Vfmadd132psVpsHpsWps
    0x000, // Vfmadd132pdVpdHpdWpd
    0x000, // Vfmadd213psVpsHpsWps
    0x000, // Vfmadd213pdVpdHpdWpd
    0x000, // Vfmadd231psVpsHpsWps
    0x000, // Vfmadd231pdVpdHpdWpd
    0x000, // Vfmadd132ssVpsHssWss
    0x000, // Vfmadd132sdVpdHsdWsd
    0x000, // Vfmadd213ssVpsHssWss
    0x000, // Vfmadd213sdVpdHsdWsd
    0x000, // Vfmadd231ssVpsHssWss
    0x000, // Vfmadd231sdVpdHsdWsd
    0x000, // Vfmaddsub132psVpsHpsWps
    0x000, // Vfmaddsub132pdVpdHpdWpd
    0x000, // Vfmaddsub213psVpsHpsWps
    0x000, // Vfmaddsub213pdVpdHpdWpd
    0x000, // Vfmaddsub231psVpsHpsWps
    0x000, // Vfmaddsub231pdVpdHpdWpd
    0x000, // Vfmsubadd132psVpsHpsWps
    0x000, // Vfmsubadd132pdVpdHpdWpd
    0x000, // Vfmsubadd213psVpsHpsWps
    0x000, // Vfmsubadd213pdVpdHpdWpd
    0x000, // Vfmsubadd231psVpsHpsWps
    0x000, // Vfmsubadd231pdVpdHpdWpd
    0x000, // Vfmsub132psVpsHpsWps
    0x000, // Vfmsub132pdVpdHpdWpd
    0x000, // Vfmsub213psVpsHpsWps
    0x000, // Vfmsub213pdVpdHpdWpd
    0x000, // Vfmsub231psVpsHpsWps
    0x000, // Vfmsub231pdVpdHpdWpd
    0x000, // Vfmsub132ssVpsHssWss
    0x000, // Vfmsub132sdVpdHsdWsd
    0x000, // Vfmsub213ssVpsHssWss
    0x000, // Vfmsub213sdVpdHsdWsd
    0x000, // Vfmsub231ssVpsHssWss
    0x000, // Vfmsub231sdVpdHsdWsd
    0x000, // Vfnmadd132psVpsHpsWps
    0x000, // Vfnmadd132pdVpdHpdWpd
    0x000, // Vfnmadd213psVpsHpsWps
    0x000, // Vfnmadd213pdVpdHpdWpd
    0x000, // Vfnmadd231psVpsHpsWps
    0x000, // Vfnmadd231pdVpdHpdWpd
    0x000, // Vfnmadd132ssVpsHssWss
    0x000, // Vfnmadd132sdVpdHsdWsd
    0x000, // Vfnmadd213ssVpsHssWss
    0x000, // Vfnmadd213sdVpdHsdWsd
    0x000, // Vfnmadd231ssVpsHssWss
    0x000, // Vfnmadd231sdVpdHsdWsd
    0x000, // Vfnmsub132psVpsHpsWps
    0x000, // Vfnmsub132pdVpdHpdWpd
    0x000, // Vfnmsub213psVpsHpsWps
    0x000, // Vfnmsub213pdVpdHpdWpd
    0x000, // Vfnmsub231psVpsHpsWps
    0x000, // Vfnmsub231pdVpdHpdWpd
    0x000, // Vfnmsub132ssVpsHssWss
    0x000, // Vfnmsub132sdVpdHsdWsd
    0x000, // Vfnmsub213ssVpsHssWss
    0x000, // Vfnmsub213sdVpdHsdWsd
    0x000, // Vfnmsub231ssVpsHssWss
    0x000, // Vfnmsub231sdVpdHsdWsd
    0x000, // VpdpbusdVdqHdqWdq
    0x000, // VpdpbusdsVdqHdqWdq
    0x000, // VpdpwssdVdqHdqWdq
    0x000, // VpdpwssdsVdqHdqWdq
    0x000, // Vpmadd52luqVdqHdqWdq
    0x000, // Vpmadd52huqVdqHdqWdq
    0x000, // VpdpbssdVdqHdqWdq
    0x000, // VpdpbssdsVdqHdqWdq
    0x000, // VpdpbsudVdqHdqWdq
    0x000, // VpdpbsudsVdqHdqWdq
    0x000, // VpdpbuudVdqHdqWdq
    0x000, // VpdpbuudsVdqHdqWdq
    0x000, // VpdpwsudVdqHdqWdq
    0x000, // VpdpwsudsVdqHdqWdq
    0x000, // VpdpwusdVdqHdqWdq
    0x000, // VpdpwusdsVdqHdqWdq
    0x000, // VpdpwuudVdqHdqWdq
    0x000, // VpdpwuudsVdqHdqWdq
    0x000, // Vbcstnebf162psVpsWw
    0x000, // Vbcstnesh2psVpsWsh
    0x000, // Vcvtneeph2psVpsWph
    0x000, // Vcvtneoph2psVpsWph
    0x000, // Vcvtneebf162psVpsWph
    0x000, // Vcvtneobf162psVpsWph
    0x000, // Vcvtneps2bf16VphWps
    0x000, // AndnGdBdEd
    0x000, // AndnGqBqEq
    0x000, // BlsiBdEd
    0x000, // BlsiBqEq
    0x000, // BlsmskBdEd
    0x000, // BlsmskBqEq
    0x000, // BlsrBdEd
    0x000, // BlsrBqEq
    0x000, // BextrGdEdBd
    0x000, // BextrGqEqBq
    0x000, // MulxGdBdEd
    0x000, // MulxGqBqEq
    0x000, // RorxGdEdIb
    0x000, // RorxGqEqIb
    0x000, // ShlxGdEdBd
    0x000, // ShlxGqEqBq
    0x000, // ShrxGdEdBd
    0x000, // ShrxGqEqBq
    0x000, // SarxGdEdBd
    0x000, // SarxGqEqBq
    0x000, // BzhiGdBdEd
    0x000, // BzhiGqBqEq
    0x000, // PextGdBdEd
    0x000, // PextGqBqEq
    0x000, // PdepGdBdEd
    0x000, // PdepGqBqEq
    0x000, // CmpbexaddEdGdBd
    0x000, // CmpbexaddEqGqBq
    0x000, // CmpbxaddEdGdBd
    0x000, // CmpbxaddEqGqBq
    0x000, // CmplexaddEdGdBd
    0x000, // CmplexaddEqGqBq
    0x000, // CmplxaddEdGdBd
    0x000, // CmplxaddEqGqBq
    0x000, // CmpnbexaddEdGdBd
    0x000, // CmpnbexaddEqGqBq
    0x000, // CmpnbxaddEdGdBd
    0x000, // CmpnbxaddEqGqBq
    0x000, // CmpnlexaddEdGdBd
    0x000, // CmpnlexaddEqGqBq
    0x000, // CmpnlxaddEdGdBd
    0x000, // CmpnlxaddEqGqBq
    0x000, // CmpnoxaddEdGdBd
    0x000, // CmpnoxaddEqGqBq
    0x000, // CmpnpxaddEdGdBd
    0x000, // CmpnpxaddEqGqBq
    0x000, // CmpnsxaddEdGdBd
    0x000, // CmpnsxaddEqGqBq
    0x000, // CmpnzxaddEdGdBd
    0x000, // CmpnzxaddEqGqBq
    0x000, // CmpoxaddEdGdBd
    0x000, // CmpoxaddEqGqBq
    0x000, // CmppxaddEdGdBd
    0x000, // CmppxaddEqGqBq
    0x000, // CmpsxaddEdGdBd
    0x000, // CmpsxaddEqGqBq
    0x000, // CmpzxaddEdGdBd
    0x000, // CmpzxaddEqGqBq
    0x000, // VfmaddsubpsVpsHpsVibWps
    0x000, // VfmaddsubpsVpsHpsWpsVib
    0x000, // VfmaddsubpdVpdHpdVibWpd
    0x000, // VfmaddsubpdVpdHpdWpdVib
    0x000, // VfmsubaddpsVpsHpsVibWps
    0x000, // VfmsubaddpsVpsHpsWpsVib
    0x000, // VfmsubaddpdVpdHpdVibWpd
    0x000, // VfmsubaddpdVpdHpdWpdVib
    0x000, // VfmaddpsVpsHpsVibWps
    0x000, // VfmaddpsVpsHpsWpsVib
    0x000, // VfmaddpdVpdHpdVibWpd
    0x000, // VfmaddpdVpdHpdWpdVib
    0x000, // VfmaddssVssHssVibWss
    0x000, // VfmaddssVssHssWssVib
    0x000, // VfmaddsdVsdHsdVibWsd
    0x000, // VfmaddsdVsdHsdWsdVib
    0x000, // VfmsubpsVpsHpsVibWps
    0x000, // VfmsubpsVpsHpsWpsVib
    0x000, // VfmsubpdVpdHpdVibWpd
    0x000, // VfmsubpdVpdHpdWpdVib
    0x000, // VfmsubssVssHssVibWss
    0x000, // VfmsubssVssHssWssVib
    0x000, // VfmsubsdVsdHsdVibWsd
    0x000, // VfmsubsdVsdHsdWsdVib
    0x000, // VfnmaddpsVpsHpsVibWps
    0x000, // VfnmaddpsVpsHpsWpsVib
    0x000, // VfnmaddpdVpdHpdVibWpd
    0x000, // VfnmaddpdVpdHpdWpdVib
    0x000, // VfnmaddssVssHssVibWss
    0x000, // VfnmaddssVssHssWssVib
    0x000, // VfnmaddsdVsdHsdVibWsd
    0x000, // VfnmaddsdVsdHsdWsdVib
    0x000, // VfnmsubpsVpsHpsVibWps
    0x000, // VfnmsubpsVpsHpsWpsVib
    0x000, // VfnmsubpdVpdHpdVibWpd
    0x000, // VfnmsubpdVpdHpdWpdVib
    0x000, // VfnmsubssVssHssVibWss
    0x000, // VfnmsubssVssHssWssVib
    0x000, // VfnmsubsdVsdHsdVibWsd
    0x000, // VfnmsubsdVsdHsdWsdVib
    0x000, // VpcmovVdqHdqVibWdq
    0x000, // VpcmovVdqHdqWdqVib
    0x000, // VppermVdqHdqVibWdq
    0x000, // VppermVdqHdqWdqVib
    0x000, // Vpermil2psVdqHdqVibWdq
    0x000, // Vpermil2psVdqHdqWdqVib
    0x000, // Vpermil2pdVdqHdqVibWdq
    0x000, // Vpermil2pdVdqHdqWdqVib
    0x000, // VpshabVdqHdqWdq
    0x000, // VpshabVdqWdqHdq
    0x000, // VpshawVdqHdqWdq
    0x000, // VpshawVdqWdqHdq
    0x000, // VpshadVdqHdqWdq
    0x000, // VpshadVdqWdqHdq
    0x000, // VpshaqVdqHdqWdq
    0x000, // VpshaqVdqWdqHdq
    0x000, // VprotbVdqHdqWdq
    0x000, // VprotbVdqWdqHdq
    0x000, // VprotwVdqHdqWdq
    0x000, // VprotwVdqWdqHdq
    0x000, // VprotdVdqHdqWdq
    0x000, // VprotdVdqWdqHdq
    0x000, // VprotqVdqHdqWdq
    0x000, // VprotqVdqWdqHdq
    0x000, // VpshlbVdqHdqWdq
    0x000, // VpshlbVdqWdqHdq
    0x000, // VpshlwVdqHdqWdq
    0x000, // VpshlwVdqWdqHdq
    0x000, // VpshldVdqHdqWdq
    0x000, // VpshldVdqWdqHdq
    0x000, // VpshlqVdqHdqWdq
    0x000, // VpshlqVdqWdqHdq
    0x000, // VpmacsswwVdqHdqWdqVib
    0x000, // VpmacsswdVdqHdqWdqVib
    0x000, // VpmacssdqlVdqHdqWdqVib
    0x000, // VpmacssddVdqHdqWdqVib
    0x000, // VpmacssdqhVdqHdqWdqVib
    0x000, // VpmacswwVdqHdqWdqVib
    0x000, // VpmacswdVdqHdqWdqVib
    0x000, // VpmacsdqlVdqHdqWdqVib
    0x000, // VpmacsddVdqHdqWdqVib
    0x000, // VpmacsdqhVdqHdqWdqVib
    0x000, // VpmadcsswdVdqHdqWdqVib
    0x000, // VpmadcswdVdqHdqWdqVib
    0x000, // VprotbVdqWdqIb
    0x000, // VprotwVdqWdqIb
    0x000, // VprotdVdqWdqIb
    0x000, // VprotqVdqWdqIb
    0x000, // VpcombVdqHdqWdqIb
    0x000, // VpcomwVdqHdqWdqIb
    0x000, // VpcomdVdqHdqWdqIb
    0x000, // VpcomqVdqHdqWdqIb
    0x000, // VpcomubVdqHdqWdqIb
    0x000, // VpcomuwVdqHdqWdqIb
    0x000, // VpcomudVdqHdqWdqIb
    0x000, // VpcomuqVdqHdqWdqIb
    0x000, // VfrczpsVpsWps
    0x000, // VfrczpdVpdWpd
    0x000, // VfrczssVssWss
    0x000, // VfrczsdVsdWsd
    0x000, // VphaddbwVdqWdq
    0x000, // VphaddbdVdqWdq
    0x000, // VphaddbqVdqWdq
    0x000, // VphaddwdVdqWdq
    0x000, // VphaddwqVdqWdq
    0x000, // VphadddqVdqWdq
    0x000, // VphaddubwVdqWdq
    0x000, // VphaddubdVdqWdq
    0x000, // VphaddubqVdqWdq
    0x000, // VphadduwdVdqWdq
    0x000, // VphadduwqVdqWdq
    0x000, // VphaddudqVdqWdq
    0x000, // VphsubbwVdqWdq
    0x000, // VphsubwdVdqWdq
    0x000, // VphsubdqVdqWdq
    0x000, // BextrGdEdId
    0x000, // BextrGqEqId
    0x000, // BlcfillBdEd
    0x000, // BlcfillBqEq
    0x000, // BlciBdEd
    0x000, // BlciBqEq
    0x000, // BlcicBdEd
    0x000, // BlcicBqEq
    0x000, // BlcmskBdEd
    0x000, // BlcmskBqEq
    0x000, // BlcsBdEd
    0x000, // BlcsBqEq
    0x000, // BlsfillBdEd
    0x000, // BlsfillBqEq
    0x000, // BlsicBdEd
    0x000, // BlsicBqEq
    0x000, // T1mskcBdEd
    0x000, // T1mskcBqEq
    0x000, // TzmskBdEd
    0x000, // TzmskBqEq
    0x000, // TzcntGwEw
    0x000, // TzcntGdEd
    0x000, // TzcntGqEq
    0x000, // LzcntGwEw
    0x000, // LzcntGdEd
    0x000, // LzcntGqEq
    0x000, // MovntssMssVss
    0x000, // MovntsdMsdVsd
    0x000, // ExtrqUdqIbIb
    0x000, // ExtrqVdqUq
    0x000, // InsertqVdqUqIbIb
    0x000, // InsertqVdqUdq
    0x000, // AdcxGdEd
    0x000, // AdoxGdEd
    0x000, // AdcxGqEq
    0x000, // AdoxGqEq
    0x000, // Stac
    0x000, // Clac
    0x000, // RdrandEw
    0x000, // RdrandEd
    0x000, // RdrandEq
    0x000, // RdseedEw
    0x000, // RdseedEd
    0x000, // RdseedEq
    0x000, // MovdiriMdGd
    0x000, // MovdiriMqGq
    0x000, // Movdir64bGdMdq
    0x000, // Movdir64bGqMdq
    0x000, // AaddEdGd
    0x000, // AandEdGd
    0x000, // AorEdGd
    0x000, // AxorEdGd
    0x000, // AaddEqGq
    0x000, // AandEqGq
    0x000, // AorEqGq
    0x000, // AxorEqGq
    0x000, // Ldtilecfg
    0x000, // Sttilecfg
    0x000, // TileloaddTnnnMdq
    0x000, // Tileloaddt1TnnnMdq
    0x000, // TileloaddrsTnnnMdq
    0x000, // Tileloaddrst1TnnnMdq
    0x000, // TilestoredMdqTnnn
    0x000, // Tilerelease
    0x000, // TilezeroTnnn
    0x000, // TdpbssdTnnnTrmTreg
    0x000, // TdpbsudTnnnTrmTreg
    0x000, // TdpbusdTnnnTrmTreg
    0x000, // TdpbuudTnnnTrmTreg
    0x000, // Tdpbf16psTnnnTrmTreg
    0x000, // Tdpfp16psTnnnTrmTreg
    0x000, // Tcmmrlfp16psTnnnTrmTreg
    0x000, // Tcmmimfp16psTnnnTrmTreg
    0x000, // Tmmultf32psTnnnTrmTreg
    0x000, // Tdpbf8psTnnnTrmTreg
    0x000, // Tdphf8psTnnnTrmTreg
    0x000, // Tdpbhf8psTnnnTrmTreg
    0x000, // Tdphbf8psTnnnTrmTreg
    0x000, // KaddwKgwKhwKew
    0x000, // KaddqKgqKhqKeq
    0x000, // KaddbKgbKhbKeb
    0x000, // KadddKgdKhdKed
    0x000, // KandwKgwKhwKew
    0x000, // KandqKgqKhqKeq
    0x000, // KandbKgbKhbKeb
    0x000, // KanddKgdKhdKed
    0x000, // KandnwKgwKhwKew
    0x000, // KandnqKgqKhqKeq
    0x000, // KandnbKgbKhbKeb
    0x000, // KandndKgdKhdKed
    0x000, // KmovwKgwKew
    0x000, // KmovqKgqKeq
    0x000, // KmovbKgbKeb
    0x000, // KmovdKgdKed
    0x000, // KmovwKewKgw
    0x000, // KmovqKeqKgq
    0x000, // KmovbKebKgb
    0x000, // KmovdKedKgd
    0x000, // KmovbGdKeb
    0x000, // KmovwGdKew
    0x000, // KmovdGdKed
    0x000, // KmovqGqKeq
    0x000, // KmovbKgbEb
    0x000, // KmovwKgwEw
    0x000, // KmovdKgdEd
    0x000, // KmovqKgqEq
    0x000, // KunpckbwKgwKhbKeb
    0x000, // KunpckwdKgdKhwKew
    0x000, // KunpckdqKgqKhdKed
    0x000, // KnotwKgwKew
    0x000, // KnotqKgqKeq
    0x000, // KnotbKgbKeb
    0x000, // KnotdKgdKed
    0x000, // KorwKgwKhwKew
    0x000, // KorqKgqKhqKeq
    0x000, // KorbKgbKhbKeb
    0x000, // KordKgdKhdKed
    0x000, // KortestwKgwKew
    0x000, // KortestqKgqKeq
    0x000, // KortestbKgbKeb
    0x000, // KortestdKgdKed
    0x000, // KshiftlbKgbKebIb
    0x000, // KshiftlwKgwKewIb
    0x000, // KshiftldKgdKedIb
    0x000, // KshiftlqKgqKeqIb
    0x000, // KshiftrbKgbKebIb
    0x000, // KshiftrwKgwKewIb
    0x000, // KshiftrdKgdKedIb
    0x000, // KshiftrqKgqKeqIb
    0x000, // KxnorwKgwKhwKew
    0x000, // KxnorqKgqKhqKeq
    0x000, // KxnorbKgbKhbKeb
    0x000, // KxnordKgdKhdKed
    0x000, // KxorwKgwKhwKew
    0x000, // KxorqKgqKhqKeq
    0x000, // KxorbKgbKhbKeb
    0x000, // KxordKgdKhdKed
    0x000, // KtestwKgwKew
    0x000, // KtestqKgqKeq
    0x000, // KtestbKgbKeb
    0x000, // KtestdKgdKed
    0x000, // RdmsrEqId
    0x000, // WrmsrnsIdEq
    0x000, // MovrsGbEb
    0x000, // MovrsGwEw
    0x000, // MovrsGdEd
    0x000, // MovrsGqEq
    0x000, // Erets
    0x000, // Eretu
    0x000, // LkgsEw
    0x080, // EvexVaddpsVpsHpsWps
    0x080, // EvexVaddpdVpdHpdWpd
    0x280, // EvexVaddssVssHpsWss
    0x280, // EvexVaddsdVsdHpdWsd
    0x080, // EvexVaddpsVpsHpsWpsKmask
    0x080, // EvexVaddpdVpdHpdWpdKmask
    0x280, // EvexVaddssVssHpsWssKmask
    0x280, // EvexVaddsdVsdHpdWsdKmask
    0x080, // EvexVsubpsVpsHpsWps
    0x080, // EvexVsubpdVpdHpdWpd
    0x280, // EvexVsubssVssHpsWss
    0x280, // EvexVsubsdVsdHpdWsd
    0x080, // EvexVsubpsVpsHpsWpsKmask
    0x080, // EvexVsubpdVpdHpdWpdKmask
    0x280, // EvexVsubssVssHpsWssKmask
    0x280, // EvexVsubsdVsdHpdWsdKmask
    0x080, // EvexVmulpsVpsHpsWps
    0x080, // EvexVmulpdVpdHpdWpd
    0x280, // EvexVmulssVssHpsWss
    0x280, // EvexVmulsdVsdHpdWsd
    0x080, // EvexVmulpsVpsHpsWpsKmask
    0x080, // EvexVmulpdVpdHpdWpdKmask
    0x280, // EvexVmulssVssHpsWssKmask
    0x280, // EvexVmulsdVsdHpdWsdKmask
    0x080, // EvexVdivpsVpsHpsWps
    0x080, // EvexVdivpdVpdHpdWpd
    0x280, // EvexVdivssVssHpsWss
    0x280, // EvexVdivsdVsdHpdWsd
    0x080, // EvexVdivpsVpsHpsWpsKmask
    0x080, // EvexVdivpdVpdHpdWpdKmask
    0x280, // EvexVdivssVssHpsWssKmask
    0x280, // EvexVdivsdVsdHpdWsdKmask
    0x080, // EvexVminpsVpsHpsWps
    0x080, // EvexVminpdVpdHpdWpd
    0x280, // EvexVminssVssHpsWss
    0x280, // EvexVminsdVsdHpdWsd
    0x080, // EvexVminpsVpsHpsWpsKmask
    0x080, // EvexVminpdVpdHpdWpdKmask
    0x280, // EvexVminssVssHpsWssKmask
    0x280, // EvexVminsdVsdHpdWsdKmask
    0x080, // EvexVmaxpsVpsHpsWps
    0x080, // EvexVmaxpdVpdHpdWpd
    0x280, // EvexVmaxssVssHpsWss
    0x280, // EvexVmaxsdVsdHpdWsd
    0x080, // EvexVmaxpsVpsHpsWpsKmask
    0x080, // EvexVmaxpdVpdHpdWpdKmask
    0x280, // EvexVmaxssVssHpsWssKmask
    0x280, // EvexVmaxsdVsdHpdWsdKmask
    0x080, // EvexVsqrtpsVpsWps
    0x080, // EvexVsqrtpdVpdWpd
    0x280, // EvexVsqrtssVssHpsWss
    0x280, // EvexVsqrtsdVsdHpdWsd
    0x080, // EvexVsqrtpsVpsWpsKmask
    0x080, // EvexVsqrtpdVpdWpdKmask
    0x280, // EvexVsqrtssVssHpsWssKmask
    0x280, // EvexVsqrtsdVsdHpdWsdKmask
    0x080, // EvexVcmppsKgwHpsWpsIb
    0x080, // EvexVcmppdKgbHpdWpdIb
    0x280, // EvexVcmpssKgbHssWssIb
    0x280, // EvexVcmpsdKgbHsdWsdIb
    0x080, // EvexVrndscalepsVpsWpsIbKmask
    0x080, // EvexVrndscalepdVpdWpdIbKmask
    0x280, // EvexVrndscalessVssHpsWssIbKmask
    0x280, // EvexVrndscalesdVsdHpdWsdIbKmask
    0x180, // EvexVunpcklpsVpsHpsWps
    0x180, // EvexVunpcklpdVpdHpdWpd
    0x180, // EvexVunpcklpsVpsHpsWpsKmask
    0x180, // EvexVunpcklpdVpdHpdWpdKmask
    0x180, // EvexVunpckhpsVpsHpsWps
    0x180, // EvexVunpckhpdVpdHpdWpd
    0x180, // EvexVunpckhpsVpsHpsWpsKmask
    0x180, // EvexVunpckhpdVpdHpdWpdKmask
    0x180, // EvexVpunpckldqVdqHdqWdq
    0x180, // EvexVpunpcklqdqVdqHdqWdq
    0x180, // EvexVpunpckldqVdqHdqWdqKmask
    0x180, // EvexVpunpcklqdqVdqHdqWdqKmask
    0x180, // EvexVpunpckhdqVdqHdqWdq
    0x180, // EvexVpunpckhqdqVdqHdqWdq
    0x180, // EvexVpunpckhdqVdqHdqWdqKmask
    0x180, // EvexVpunpckhqdqVdqHdqWdqKmask
    0x180, // EvexVpmuldqVdqHdqWdq
    0x180, // EvexVpmuludqVdqHdqWdq
    0x180, // EvexVpmuldqVdqHdqWdqKmask
    0x180, // EvexVpmuludqVdqHdqWdqKmask
    0x280, // EvexVucomissVssWss
    0x280, // EvexVcomissVssWss
    0x280, // EvexVucomisdVsdWsd
    0x280, // EvexVcomisdVsdWsd
    0x280, // EvexVcvtss2sdVsdWss
    0x280, // EvexVcvtsd2ssVssWsd
    0x080, // EvexVcvtps2pdVpdWps
    0x080, // EvexVcvtpd2psVpsWpd
    0x280, // EvexVcvtss2sdVsdWssKmask
    0x280, // EvexVcvtsd2ssVssWsdKmask
    0x080, // EvexVcvtps2pdVpdWpsKmask
    0x080, // EvexVcvtpd2psVpsWpdKmask
    0x080, // EvexVcvtps2dqVdqWps
    0x080, // EvexVcvtps2dqVdqWpsKmask
    0x080, // EvexVcvttps2dqVdqWps
    0x080, // EvexVcvttps2dqVdqWpsKmask
    0x080, // EvexVcvtpd2dqVdqWpd
    0x080, // EvexVcvtpd2dqVdqWpdKmask
    0x080, // EvexVcvttpd2dqVdqWpd
    0x080, // EvexVcvttpd2dqVdqWpdKmask
    0x280, // EvexVcvtph2psVpsWps
    0x280, // EvexVcvtph2psVpsWpsKmask
    0x280, // EvexVcvtps2phWpsVpsIb
    0x280, // EvexVcvtps2phWpsVpsIbKmask
    0x180, // EvexVcvtneps2bf16VphWpsKmask
    0x180, // EvexVcvtne2ps2bf16VphHpsWpsKmask
    0x180, // EvexVdpbf16psVpsHdqWdqKmask
    0x380, // EvexVmovapsVpsWps
    0x380, // EvexVmovapsVpsWpsKmask
    0x380, // EvexVmovapsWpsVps
    0x380, // EvexVmovapsWpsVpsKmask
    0x380, // EvexVmovapdVpdWpd
    0x380, // EvexVmovapdVpdWpdKmask
    0x380, // EvexVmovapdWpdVpd
    0x380, // EvexVmovapdWpdVpdKmask
    0x380, // EvexVmovupsVpsWps
    0x380, // EvexVmovupsVpsWpsKmask
    0x380, // EvexVmovupsWpsVps
    0x380, // EvexVmovupsWpsVpsKmask
    0x380, // EvexVmovupdVpdWpd
    0x380, // EvexVmovupdVpdWpdKmask
    0x380, // EvexVmovupdWpdVpd
    0x380, // EvexVmovupdWpdVpdKmask
    0x180, // EvexVmovsdVsdHpdWsd
    0x180, // EvexVmovssVssHpsWss
    0x180, // EvexVmovsdWsdHpdVsd
    0x180, // EvexVmovssWssHpsVss
    0x280, // EvexVmovsdVsdWsd
    0x280, // EvexVmovssVssWss
    0x280, // EvexVmovsdWsdVsd
    0x280, // EvexVmovssWssVss
    0x180, // EvexVmovsdVsdHpdWsdKmask
    0x180, // EvexVmovssVssHpsWssKmask
    0x180, // EvexVmovsdWsdHpdVsdKmask
    0x180, // EvexVmovssWssHpsVssKmask
    0x280, // EvexVmovsdVsdWsdKmask
    0x280, // EvexVmovssVssWssKmask
    0x280, // EvexVmovsdWsdVsdKmask
    0x280, // EvexVmovssWssVssKmask
    0x380, // EvexVpabsbVdqWdq
    0x380, // EvexVpabswVdqWdq
    0x180, // EvexVpabsdVdqWdq
    0x180, // EvexVpabsqVdqWdq
    0x380, // EvexVpabsbVdqWdqKmask
    0x380, // EvexVpabswVdqWdqKmask
    0x180, // EvexVpabsdVdqWdqKmask
    0x180, // EvexVpabsqVdqWdqKmask
    0x280, // EvexVmovntdqaVdqMdq
    0x280, // EvexVmovntpsMpsVps
    0x280, // EvexVmovntpdMpdVpd
    0x280, // EvexVmovntdqMdqVdq
    0x380, // EvexVpcmpeqbKgqHdqWdq
    0x380, // EvexVpcmpeqwKgdHdqWdq
    0x380, // EvexVpcmpgtbKgqHdqWdq
    0x380, // EvexVpcmpgtwKgdHdqWdq
    0x180, // EvexVpcmpeqdKgwHdqWdq
    0x180, // EvexVpcmpeqqKgbHdqWdq
    0x180, // EvexVpcmpgtdKgwHdqWdq
    0x180, // EvexVpcmpgtqKgbHdqWdq
    0x380, // EvexVpsrlwVdqHdqWdq
    0x380, // EvexVpsrlwVdqHdqWdqKmask
    0x380, // EvexVpsrawVdqHdqWdq
    0x380, // EvexVpsrawVdqHdqWdqKmask
    0x380, // EvexVpsllwVdqHdqWdq
    0x380, // EvexVpsllwVdqHdqWdqKmask
    0x380, // EvexVpsrlwUdqIb
    0x380, // EvexVpsrlwUdqIbKmask
    0x380, // EvexVpsllwUdqIb
    0x380, // EvexVpsllwUdqIbKmask
    0x380, // EvexVpsrawUdqIb
    0x380, // EvexVpsrawUdqIbKmask
    0x380, // EvexVpsrldVdqHdqWdq
    0x380, // EvexVpsrlqVdqHdqWdq
    0x380, // EvexVpsrldVdqHdqWdqKmask
    0x380, // EvexVpsrlqVdqHdqWdqKmask
    0x380, // EvexVpslldVdqHdqWdq
    0x380, // EvexVpsllqVdqHdqWdq
    0x380, // EvexVpslldVdqHdqWdqKmask
    0x380, // EvexVpsllqVdqHdqWdqKmask
    0x180, // EvexVpsrldUdqIb
    0x180, // EvexVpsrldUdqIbKmask
    0x180, // EvexVpsrlqUdqIb
    0x180, // EvexVpsrlqUdqIbKmask
    0x180, // EvexVpslldUdqIb
    0x180, // EvexVpslldUdqIbKmask
    0x180, // EvexVpsllqUdqIb
    0x180, // EvexVpsllqUdqIbKmask
    0x380, // EvexVpshufbVdqHdqWdq
    0x380, // EvexVpshufbVdqHdqWdqKmask
    0x180, // EvexVpermqVdqWdqIbKmask
    0x180, // EvexVpermpdVpdWpdIbKmask
    0x180, // EvexVshufpsVpsHpsWpsIb
    0x180, // EvexVshufpdVpdHpdWpdIb
    0x180, // EvexVshufpsVpsHpsWpsIbKmask
    0x180, // EvexVshufpdVpdHpdWpdIbKmask
    0x180, // EvexVpermilpsVpsHpsWps
    0x180, // EvexVpermilpdVpdHpdWpd
    0x180, // EvexVpermilpsVpsHpsWpsKmask
    0x180, // EvexVpermilpdVpdHpdWpdKmask
    0x180, // EvexVpermilpsVpsWpsIb
    0x180, // EvexVpermilpdVpdWpdIb
    0x180, // EvexVpermilpsVpsWpsIbKmask
    0x180, // EvexVpermilpdVpdWpdIbKmask
    0x180, // EvexVpshufdVdqWdqIb
    0x180, // EvexVpshufdVdqWdqIbKmask
    0x380, // EvexVpshuflwVdqWdqIb
    0x380, // EvexVpshuflwVdqWdqIbKmask
    0x380, // EvexVpshufhwVdqWdqIb
    0x380, // EvexVpshufhwVdqWdqIbKmask
    0x380, // EvexVpbroadcastbVdqEb
    0x380, // EvexVpbroadcastbVdqEbKmask
    0x380, // EvexVpbroadcastwVdqEw
    0x380, // EvexVpbroadcastwVdqEwKmask
    0x380, // EvexVpbroadcastdVdqEd
    0x380, // EvexVpbroadcastdVdqEdKmask
    0x380, // EvexVpbroadcastqVdqEq
    0x380, // EvexVpbroadcastqVdqEqKmask
    0x380, // EvexVpbroadcastbVdqWb
    0x380, // EvexVpbroadcastbVdqWbKmask
    0x380, // EvexVpbroadcastwVdqWw
    0x380, // EvexVpbroadcastwVdqWwKmask
    0x380, // EvexVpbroadcastdVdqWd
    0x380, // EvexVpbroadcastdVdqWdKmask
    0x380, // EvexVpbroadcastqVdqWq
    0x380, // EvexVpbroadcastqVdqWqKmask
    0x380, // EvexVbroadcastssVpsWss
    0x380, // EvexVbroadcastssVpsWssKmask
    0x380, // EvexVbroadcastsdVpdWsd
    0x380, // EvexVbroadcastsdVpdWsdKmask
    0x380, // EvexVmovqWqVq
    0x380, // EvexVmovqVqWq
    0x380, // EvexVinsertpsVpsWssIb
    0x380, // EvexVextractpsEdVpsIb
    0x380, // EvexVmovlpsVpsHpsMq
    0x380, // EvexVmovhlpsVpsHpsWps
    0x380, // EvexVmovhpsVpsHpsMq
    0x380, // EvexVmovlhpsVpsHpsWps
    0x380, // EvexVmovlpsMqVps
    0x380, // EvexVmovhpsMqVps
    0x380, // EvexVmovlpdMqVsd
    0x380, // EvexVmovhpdMqVsd
    0x380, // EvexVmovlpdVpdHpdMq
    0x380, // EvexVmovhpdVpdHpdMq
    0x380, // EvexVmovddupVpdWpd
    0x380, // EvexVmovsldupVpsWps
    0x380, // EvexVmovshdupVpsWps
    0x380, // EvexVmovddupVpdWpdKmask
    0x380, // EvexVmovsldupVpsWpsKmask
    0x380, // EvexVmovshdupVpsWpsKmask
    0x380, // EvexVpmovqbWdqVdq
    0x380, // EvexVpmovdbWdqVdq
    0x380, // EvexVpmovwbWdqVdq
    0x380, // EvexVpmovdwWdqVdq
    0x380, // EvexVpmovqwWdqVdq
    0x380, // EvexVpmovqdWdqVdq
    0x380, // EvexVpmovqbWdqVdqKmask
    0x380, // EvexVpmovdbWdqVdqKmask
    0x380, // EvexVpmovwbWdqVdqKmask
    0x380, // EvexVpmovdwWdqVdqKmask
    0x380, // EvexVpmovqwWdqVdqKmask
    0x380, // EvexVpmovqdWdqVdqKmask
    0x380, // EvexVpmovusqbWdqVdq
    0x380, // EvexVpmovusdbWdqVdq
    0x380, // EvexVpmovuswbWdqVdq
    0x380, // EvexVpmovusdwWdqVdq
    0x380, // EvexVpmovusqwWdqVdq
    0x380, // EvexVpmovusqdWdqVdq
    0x380, // EvexVpmovusqbWdqVdqKmask
    0x380, // EvexVpmovusdbWdqVdqKmask
    0x380, // EvexVpmovuswbWdqVdqKmask
    0x380, // EvexVpmovusdwWdqVdqKmask
    0x380, // EvexVpmovusqwWdqVdqKmask
    0x380, // EvexVpmovusqdWdqVdqKmask
    0x380, // EvexVpmovsqbWdqVdq
    0x380, // EvexVpmovsdbWdqVdq
    0x380, // EvexVpmovswbWdqVdq
    0x380, // EvexVpmovsdwWdqVdq
    0x380, // EvexVpmovsqwWdqVdq
    0x380, // EvexVpmovsqdWdqVdq
    0x380, // EvexVpmovsqbWdqVdqKmask
    0x380, // EvexVpmovsdbWdqVdqKmask
    0x380, // EvexVpmovswbWdqVdqKmask
    0x380, // EvexVpmovsdwWdqVdqKmask
    0x380, // EvexVpmovsqwWdqVdqKmask
    0x380, // EvexVpmovsqdWdqVdqKmask
    0x380, // EvexVpmovsxbwVdqWdq
    0x380, // EvexVpmovsxbdVdqWdq
    0x380, // EvexVpmovsxbqVdqWdq
    0x380, // EvexVpmovsxwdVdqWdq
    0x380, // EvexVpmovsxwqVdqWdq
    0x380, // EvexVpmovsxdqVdqWdq
    0x380, // EvexVpmovsxbwVdqWdqKmask
    0x380, // EvexVpmovsxbdVdqWdqKmask
    0x380, // EvexVpmovsxbqVdqWdqKmask
    0x380, // EvexVpmovsxwdVdqWdqKmask
    0x380, // EvexVpmovsxwqVdqWdqKmask
    0x380, // EvexVpmovsxdqVdqWdqKmask
    0x380, // EvexVpmovzxbwVdqWdq
    0x380, // EvexVpmovzxbdVdqWdq
    0x380, // EvexVpmovzxbqVdqWdq
    0x380, // EvexVpmovzxwdVdqWdq
    0x380, // EvexVpmovzxwqVdqWdq
    0x380, // EvexVpmovzxdqVdqWdq
    0x380, // EvexVpmovzxbwVdqWdqKmask
    0x380, // EvexVpmovzxbdVdqWdqKmask
    0x380, // EvexVpmovzxbqVdqWdqKmask
    0x380, // EvexVpmovzxwdVdqWdqKmask
    0x380, // EvexVpmovzxwqVdqWdqKmask
    0x380, // EvexVpmovzxdqVdqWdqKmask
    0x380, // EvexVpsubbVdqHdqWdq
    0x380, // EvexVpsubsbVdqHdqWdq
    0x380, // EvexVpsubusbVdqHdqWdq
    0x380, // EvexVpsubwVdqHdqWdq
    0x380, // EvexVpsubswVdqHdqWdq
    0x380, // EvexVpsubuswVdqHdqWdq
    0x380, // EvexVpaddbVdqHdqWdq
    0x380, // EvexVpaddsbVdqHdqWdq
    0x380, // EvexVpaddusbVdqHdqWdq
    0x380, // EvexVpaddwVdqHdqWdq
    0x380, // EvexVpaddswVdqHdqWdq
    0x380, // EvexVpadduswVdqHdqWdq
    0x380, // EvexVpsubbVdqHdqWdqKmask
    0x380, // EvexVpsubsbVdqHdqWdqKmask
    0x380, // EvexVpsubusbVdqHdqWdqKmask
    0x380, // EvexVpsubwVdqHdqWdqKmask
    0x380, // EvexVpsubswVdqHdqWdqKmask
    0x380, // EvexVpsubuswVdqHdqWdqKmask
    0x380, // EvexVpaddbVdqHdqWdqKmask
    0x380, // EvexVpaddsbVdqHdqWdqKmask
    0x380, // EvexVpaddusbVdqHdqWdqKmask
    0x380, // EvexVpaddwVdqHdqWdqKmask
    0x380, // EvexVpaddswVdqHdqWdqKmask
    0x380, // EvexVpadduswVdqHdqWdqKmask
    0x380, // EvexVpminsbVdqHdqWdq
    0x380, // EvexVpminubVdqHdqWdq
    0x380, // EvexVpmaxubVdqHdqWdq
    0x380, // EvexVpmaxsbVdqHdqWdq
    0x380, // EvexVpminswVdqHdqWdq
    0x380, // EvexVpminuwVdqHdqWdq
    0x380, // EvexVpmaxswVdqHdqWdq
    0x380, // EvexVpmaxuwVdqHdqWdq
    0x380, // EvexVpminsbVdqHdqWdqKmask
    0x380, // EvexVpminubVdqHdqWdqKmask
    0x380, // EvexVpmaxubVdqHdqWdqKmask
    0x380, // EvexVpmaxsbVdqHdqWdqKmask
    0x380, // EvexVpminswVdqHdqWdqKmask
    0x380, // EvexVpminuwVdqHdqWdqKmask
    0x380, // EvexVpmaxswVdqHdqWdqKmask
    0x380, // EvexVpmaxuwVdqHdqWdqKmask
    0x380, // EvexVpacksswbVdqHdqWdq
    0x380, // EvexVpacksswbVdqHdqWdqKmask
    0x380, // EvexVpackuswbVdqHdqWdq
    0x380, // EvexVpackuswbVdqHdqWdqKmask
    0x180, // EvexVpackssdwVdqHdqWdq
    0x180, // EvexVpackssdwVdqHdqWdqKmask
    0x180, // EvexVpackusdwVdqHdqWdq
    0x180, // EvexVpackusdwVdqHdqWdqKmask
    0x380, // EvexVpunpcklbwVdqHdqWdq
    0x380, // EvexVpunpckhbwVdqHdqWdq
    0x380, // EvexVpunpcklbwVdqHdqWdqKmask
    0x380, // EvexVpunpckhbwVdqHdqWdqKmask
    0x380, // EvexVpunpcklwdVdqHdqWdq
    0x380, // EvexVpunpckhwdVdqHdqWdq
    0x380, // EvexVpunpcklwdVdqHdqWdqKmask
    0x380, // EvexVpunpckhwdVdqHdqWdqKmask
    0x380, // EvexVpavgbVdqHdqWdq
    0x380, // EvexVpavgwVdqHdqWdq
    0x380, // EvexVpavgbVdqHdqWdqKmask
    0x380, // EvexVpavgwVdqHdqWdqKmask
    0x380, // EvexVpmaddubswVdqHdqWdq
    0x380, // EvexVpmaddubswVdqHdqWdqKmask
    0x380, // EvexVpmullwVdqHdqWdq
    0x380, // EvexVpmulhwVdqHdqWdq
    0x380, // EvexVpmulhuwVdqHdqWdq
    0x380, // EvexVpmulhrswVdqHdqWdq
    0x380, // EvexVpmullwVdqHdqWdqKmask
    0x380, // EvexVpmulhwVdqHdqWdqKmask
    0x380, // EvexVpmulhuwVdqHdqWdqKmask
    0x380, // EvexVpmulhrswVdqHdqWdqKmask
    0x380, // EvexVpsrldqUdqIb
    0x380, // EvexVpslldqUdqIb
    0x380, // EvexVpsadbwVdqHdqWdq
    0x380, // EvexVpmaddwdVdqHdqWdq
    0x380, // EvexVpmaddwdVdqHdqWdqKmask
    0x180, // EvexVpmadd52luqVdqHdqWdq
    0x180, // EvexVpmadd52luqVdqHdqWdqKmask
    0x180, // EvexVpmadd52huqVdqHdqWdq
    0x180, // EvexVpmadd52huqVdqHdqWdqKmask
    0x000, // EvexVpmultishiftqbVdqHdqWdq
    0x180, // EvexVpmultishiftqbVdqHdqWdqKmask
    0x380, // EvexVpermbVdqHdqWdqKmask
    0x380, // EvexVpermwVdqHdqWdqKmask
    0x380, // EvexVpermt2bVdqHdqWdqKmask
    0x380, // EvexVpermt2wVdqHdqWdqKmask
    0x380, // EvexVpermi2bVdqHdqWdqKmask
    0x380, // EvexVpermi2wVdqHdqWdqKmask
    0x380, // EvexVinsertf32x4VpsHpsWpsIb
    0x380, // EvexVinsertf64x2VpdHpdWpdIb
    0x380, // EvexVinsertf32x4VpsHpsWpsIbKmask
    0x380, // EvexVinsertf64x2VpdHpdWpdIbKmask
    0x380, // EvexVinsertf32x8VpsHpsWpsIb
    0x380, // EvexVinsertf64x4VpdHpdWpdIb
    0x380, // EvexVinsertf32x8VpsHpsWpsIbKmask
    0x380, // EvexVinsertf64x4VpdHpdWpdIbKmask
    0x380, // EvexVinserti32x4VdqHdqWdqIb
    0x380, // EvexVinserti64x2VdqHdqWdqIb
    0x380, // EvexVinserti32x4VdqHdqWdqIbKmask
    0x380, // EvexVinserti64x2VdqHdqWdqIbKmask
    0x380, // EvexVinserti32x8VdqHdqWdqIb
    0x380, // EvexVinserti64x4VdqHdqWdqIb
    0x380, // EvexVinserti32x8VdqHdqWdqIbKmask
    0x380, // EvexVinserti64x4VdqHdqWdqIbKmask
    0x380, // EvexVextractf32x4WpsVpsIb
    0x380, // EvexVextractf64x2WpdVpdIb
    0x380, // EvexVextractf32x4WpsVpsIbKmask
    0x380, // EvexVextractf64x2WpdVpdIbKmask
    0x380, // EvexVextractf32x8WpsVpsIb
    0x380, // EvexVextractf64x4WpdVpdIb
    0x380, // EvexVextractf32x8WpsVpsIbKmask
    0x380, // EvexVextractf64x4WpdVpdIbKmask
    0x380, // EvexVextracti32x4WdqVdqIb
    0x380, // EvexVextracti64x2WdqVdqIb
    0x380, // EvexVextracti32x4WdqVdqIbKmask
    0x380, // EvexVextracti64x2WdqVdqIbKmask
    0x380, // EvexVextracti32x8WdqVdqIb
    0x380, // EvexVextracti64x4WdqVdqIb
    0x380, // EvexVextracti32x8WdqVdqIbKmask
    0x380, // EvexVextracti64x4WdqVdqIbKmask
    0x380, // EvexVbroadcastf32x2VpsWq
    0x380, // EvexVbroadcastf32x2VpsWqKmask
    0x380, // EvexVbroadcasti32x2VdqWq
    0x380, // EvexVbroadcasti32x2VdqWqKmask
    0x380, // EvexVbroadcastf32x4VpsWps
    0x380, // EvexVbroadcastf64x2VpdWpd
    0x380, // EvexVbroadcastf32x4VpsWpsKmask
    0x380, // EvexVbroadcastf64x2VpdWpdKmask
    0x380, // EvexVbroadcastf32x8VpsWps
    0x380, // EvexVbroadcastf64x4VpdWpd
    0x380, // EvexVbroadcastf32x8VpsWpsKmask
    0x380, // EvexVbroadcastf64x4VpdWpdKmask
    0x380, // EvexVbroadcasti32x4VdqWdq
    0x380, // EvexVbroadcasti64x2VdqWdq
    0x380, // EvexVbroadcasti32x4VdqWdqKmask
    0x380, // EvexVbroadcasti64x2VdqWdqKmask
    0x380, // EvexVbroadcasti32x8VdqWdq
    0x380, // EvexVbroadcasti64x4VdqWdq
    0x380, // EvexVbroadcasti32x8VdqWdqKmask
    0x380, // EvexVbroadcasti64x4VdqWdqKmask
    0x180, // EvexVpmulldVdqHdqWdq
    0x180, // EvexVpmullqVdqHdqWdq
    0x180, // EvexVpmulldVdqHdqWdqKmask
    0x180, // EvexVpmullqVdqHdqWdqKmask
    0x180, // EvexVpadddVdqHdqWdq
    0x180, // EvexVpaddqVdqHdqWdq
    0x180, // EvexVpadddVdqHdqWdqKmask
    0x180, // EvexVpaddqVdqHdqWdqKmask
    0x180, // EvexVpsubdVdqHdqWdq
    0x180, // EvexVpsubqVdqHdqWdq
    0x180, // EvexVpsubdVdqHdqWdqKmask
    0x180, // EvexVpsubqVdqHdqWdqKmask
    0x180, // EvexVpanddVdqHdqWdq
    0x180, // EvexVpandqVdqHdqWdq
    0x180, // EvexVpanddVdqHdqWdqKmask
    0x180, // EvexVpandqVdqHdqWdqKmask
    0x180, // EvexVpandndVdqHdqWdq
    0x180, // EvexVpandnqVdqHdqWdq
    0x180, // EvexVpandndVdqHdqWdqKmask
    0x180, // EvexVpandnqVdqHdqWdqKmask
    0x180, // EvexVpordVdqHdqWdq
    0x180, // EvexVporqVdqHdqWdq
    0x180, // EvexVpordVdqHdqWdqKmask
    0x180, // EvexVporqVdqHdqWdqKmask
    0x180, // EvexVpxordVdqHdqWdq
    0x180, // EvexVpxorqVdqHdqWdq
    0x180, // EvexVpxordVdqHdqWdqKmask
    0x180, // EvexVpxorqVdqHdqWdqKmask
    0x180, // EvexVandpsVpsHpsWps
    0x180, // EvexVandpdVpdHpdWpd
    0x180, // EvexVandpsVpsHpsWpsKmask
    0x180, // EvexVandpdVpdHpdWpdKmask
    0x180, // EvexVandnpsVpsHpsWps
    0x180, // EvexVandnpdVpdHpdWpd
    0x180, // EvexVandnpsVpsHpsWpsKmask
    0x180, // EvexVandnpdVpdHpdWpdKmask
    0x180, // EvexVorpsVpsHpsWps
    0x180, // EvexVorpdVpdHpdWpd
    0x180, // EvexVorpsVpsHpsWpsKmask
    0x180, // EvexVorpdVpdHpdWpdKmask
    0x180, // EvexVxorpsVpsHpsWps
    0x180, // EvexVxorpdVpdHpdWpd
    0x180, // EvexVxorpsVpsHpsWpsKmask
    0x180, // EvexVxorpdVpdHpdWpdKmask
    0x180, // EvexVpmaxsdVdqHdqWdq
    0x180, // EvexVpmaxsqVdqHdqWdq
    0x180, // EvexVpmaxsdVdqHdqWdqKmask
    0x180, // EvexVpmaxsqVdqHdqWdqKmask
    0x180, // EvexVpmaxudVdqHdqWdq
    0x180, // EvexVpmaxuqVdqHdqWdq
    0x180, // EvexVpmaxudVdqHdqWdqKmask
    0x180, // EvexVpmaxuqVdqHdqWdqKmask
    0x180, // EvexVpminsdVdqHdqWdq
    0x180, // EvexVpminsqVdqHdqWdq
    0x180, // EvexVpminsdVdqHdqWdqKmask
    0x180, // EvexVpminsqVdqHdqWdqKmask
    0x180, // EvexVpminudVdqHdqWdq
    0x180, // EvexVpminuqVdqHdqWdq
    0x180, // EvexVpminudVdqHdqWdqKmask
    0x180, // EvexVpminuqVdqHdqWdqKmask
    0x180, // EvexValigndVdqHdqWdqIbKmask
    0x180, // EvexValignqVdqHdqWdqIbKmask
    0x380, // EvexVpalignrVdqHdqWdqIb
    0x380, // EvexVpalignrVdqHdqWdqIbKmask
    0x380, // EvexVdbpsadbwVdqHdqWdqIbKmask
    0x380, // EvexVpsrlvwVdqHdqWdq
    0x180, // EvexVpsrlvdVdqHdqWdq
    0x180, // EvexVpsrlvqVdqHdqWdq
    0x380, // EvexVpsravwVdqHdqWdq
    0x180, // EvexVpsravdVdqHdqWdq
    0x180, // EvexVpsravqVdqHdqWdq
    0x380, // EvexVpsllvwVdqHdqWdq
    0x180, // EvexVpsllvdVdqHdqWdq
    0x180, // EvexVpsllvqVdqHdqWdq
    0x180, // EvexVprolvdVdqHdqWdq
    0x180, // EvexVprolvqVdqHdqWdq
    0x180, // EvexVprorvdVdqHdqWdq
    0x180, // EvexVprorvqVdqHdqWdq
    0x380, // EvexVpsrlvwVdqHdqWdqKmask
    0x180, // EvexVpsrlvdVdqHdqWdqKmask
    0x180, // EvexVpsrlvqVdqHdqWdqKmask
    0x380, // EvexVpsravwVdqHdqWdqKmask
    0x180, // EvexVpsravdVdqHdqWdqKmask
    0x180, // EvexVpsravqVdqHdqWdqKmask
    0x380, // EvexVpsllvwVdqHdqWdqKmask
    0x180, // EvexVpsllvdVdqHdqWdqKmask
    0x180, // EvexVpsllvqVdqHdqWdqKmask
    0x180, // EvexVprolvdVdqHdqWdqKmask
    0x180, // EvexVprolvqVdqHdqWdqKmask
    0x180, // EvexVprorvdVdqHdqWdqKmask
    0x180, // EvexVprorvqVdqHdqWdqKmask
    0x380, // EvexVpsradVdqHdqWdq
    0x380, // EvexVpsraqVdqHdqWdq
    0x380, // EvexVpsradVdqHdqWdqKmask
    0x380, // EvexVpsraqVdqHdqWdqKmask
    0x180, // EvexVpsradUdqIb
    0x180, // EvexVpsraqUdqIb
    0x180, // EvexVprordUdqIb
    0x180, // EvexVprorqUdqIb
    0x180, // EvexVproldUdqIb
    0x180, // EvexVprolqUdqIb
    0x180, // EvexVpsradUdqIbKmask
    0x180, // EvexVpsraqUdqIbKmask
    0x180, // EvexVprordUdqIbKmask
    0x180, // EvexVprorqUdqIbKmask
    0x180, // EvexVproldUdqIbKmask
    0x180, // EvexVprolqUdqIbKmask
    0x380, // EvexVmovdqu8VdqWdq
    0x380, // EvexVmovdqu16VdqWdq
    0x380, // EvexVmovdqu8VdqWdqKmask
    0x380, // EvexVmovdqu16VdqWdqKmask
    0x380, // EvexVmovdqu8WdqVdq
    0x380, // EvexVmovdqu16WdqVdq
    0x380, // EvexVmovdqu8WdqVdqKmask
    0x380, // EvexVmovdqu16WdqVdqKmask
    0x380, // EvexVmovdqu32VdqWdq
    0x380, // EvexVmovdqu64VdqWdq
    0x380, // EvexVmovdqu32VdqWdqKmask
    0x380, // EvexVmovdqu64VdqWdqKmask
    0x380, // EvexVmovdqu32WdqVdq
    0x380, // EvexVmovdqu64WdqVdq
    0x380, // EvexVmovdqu32WdqVdqKmask
    0x380, // EvexVmovdqu64WdqVdqKmask
    0x380, // EvexVmovdqa32VdqWdq
    0x380, // EvexVmovdqa64VdqWdq
    0x380, // EvexVmovdqa32VdqWdqKmask
    0x380, // EvexVmovdqa64VdqWdqKmask
    0x380, // EvexVmovdqa32WdqVdq
    0x380, // EvexVmovdqa64WdqVdq
    0x380, // EvexVmovdqa32WdqVdqKmask
    0x380, // EvexVmovdqa64WdqVdqKmask
    0x080, // EvexVrangepsVpsHpsWpsIbKmask
    0x080, // EvexVrangepdVpdHpdWpdIbKmask
    0x280, // EvexVrangessVssHpsWssIbKmask
    0x280, // EvexVrangesdVsdHpdWsdIbKmask
    0x080, // EvexVgetexppsVpsWps
    0x080, // EvexVgetexppdVpdWpd
    0x280, // EvexVgetexpssVssHpsWss
    0x280, // EvexVgetexpsdVsdHpdWsd
    0x080, // EvexVgetexppsVpsWpsKmask
    0x080, // EvexVgetexppdVpdWpdKmask
    0x280, // EvexVgetexpssVssHpsWssKmask
    0x280, // EvexVgetexpsdVsdHpdWsdKmask
    0x080, // EvexVgetmantpsVpsWpsIbKmask
    0x080, // EvexVgetmantpdVpdWpdIbKmask
    0x280, // EvexVgetmantssVssHpsWssIbKmask
    0x280, // EvexVgetmantsdVsdHpdWsdIbKmask
    0x080, // EvexVscalefpsVpsHpsWps
    0x080, // EvexVscalefpdVpdHpdWpd
    0x280, // EvexVscalefssVssHpsWss
    0x280, // EvexVscalefsdVsdHpdWsd
    0x080, // EvexVscalefpsVpsHpsWpsKmask
    0x080, // EvexVscalefpdVpdHpdWpdKmask
    0x280, // EvexVscalefssVssHpsWssKmask
    0x280, // EvexVscalefsdVsdHpdWsdKmask
    0x180, // EvexVrcp14psVpsWpsKmask
    0x180, // EvexVrcp14pdVpdWpdKmask
    0x380, // EvexVrcp14ssVssHpsWssKmask
    0x380, // EvexVrcp14sdVsdHpdWsdKmask
    0x180, // EvexVrsqrt14psVpsWpsKmask
    0x180, // EvexVrsqrt14pdVpdWpdKmask
    0x380, // EvexVrsqrt14ssVssHpsWssKmask
    0x380, // EvexVrsqrt14sdVsdHpdWsdKmask
    0x080, // EvexVcvtps2uqqVdqWps
    0x080, // EvexVcvtpd2uqqVdqWpd
    0x080, // EvexVcvtps2uqqVdqWpsKmask
    0x080, // EvexVcvtpd2uqqVdqWpdKmask
    0x080, // EvexVcvttps2uqqVdqWps
    0x080, // EvexVcvttps2uqqVdqWpsKmask
    0x080, // EvexVcvttpd2uqqVdqWpd
    0x080, // EvexVcvttpd2uqqVdqWpdKmask
    0x080, // EvexVcvtps2qqVdqWps
    0x080, // EvexVcvtps2qqVdqWpsKmask
    0x080, // EvexVcvtpd2qqVdqWpd
    0x080, // EvexVcvtpd2qqVdqWpdKmask
    0x080, // EvexVcvttps2qqVdqWps
    0x080, // EvexVcvttps2qqVdqWpsKmask
    0x080, // EvexVcvttpd2qqVdqWpd
    0x080, // EvexVcvttpd2qqVdqWpdKmask
    0x080, // EvexVcvttps2udqVdqWps
    0x080, // EvexVcvttpd2udqVdqWpd
    0x080, // EvexVcvttps2udqVdqWpsKmask
    0x080, // EvexVcvttpd2udqVdqWpdKmask
    0x080, // EvexVcvtps2udqVdqWps
    0x080, // EvexVcvtpd2udqVdqWpd
    0x080, // EvexVcvtps2udqVdqWpsKmask
    0x080, // EvexVcvtpd2udqVdqWpdKmask
    0x080, // EvexVcvtudq2pdVpdWdq
    0x080, // EvexVcvtudq2pdVpdWdqKmask
    0x080, // EvexVcvtuqq2pdVpdWdq
    0x080, // EvexVcvtuqq2pdVpdWdqKmask
    0x080, // EvexVcvtudq2psVpsWdq
    0x080, // EvexVcvtudq2psVpsWdqKmask
    0x080, // EvexVcvtuqq2psVpsWdq
    0x080, // EvexVcvtuqq2psVpsWdqKmask
    0x080, // EvexVcvtdq2pdVpdWdq
    0x080, // EvexVcvtdq2pdVpdWdqKmask
    0x080, // EvexVcvtqq2pdVpdWdq
    0x080, // EvexVcvtqq2pdVpdWdqKmask
    0x080, // EvexVcvtdq2psVpsWdq
    0x080, // EvexVcvtdq2psVpsWdqKmask
    0x080, // EvexVcvtqq2psVpsWdq
    0x080, // EvexVcvtqq2psVpsWdqKmask
    0x080, // EvexVfmadd132psVpsHpsWps
    0x080, // EvexVfmadd132pdVpdHpdWpd
    0x080, // EvexVfmadd213psVpsHpsWps
    0x080, // EvexVfmadd213pdVpdHpdWpd
    0x080, // EvexVfmadd231psVpsHpsWps
    0x080, // EvexVfmadd231pdVpdHpdWpd
    0x080, // EvexVfmadd132psVpsHpsWpsKmask
    0x080, // EvexVfmadd132pdVpdHpdWpdKmask
    0x080, // EvexVfmadd213psVpsHpsWpsKmask
    0x080, // EvexVfmadd213pdVpdHpdWpdKmask
    0x080, // EvexVfmadd231psVpsHpsWpsKmask
    0x080, // EvexVfmadd231pdVpdHpdWpdKmask
    0x280, // EvexVfmadd132ssVpsHssWss
    0x280, // EvexVfmadd132sdVpdHsdWsd
    0x280, // EvexVfmadd213ssVpsHssWss
    0x280, // EvexVfmadd213sdVpdHsdWsd
    0x280, // EvexVfmadd231ssVpsHssWss
    0x280, // EvexVfmadd231sdVpdHsdWsd
    0x280, // EvexVfmadd132ssVpsHssWssKmask
    0x280, // EvexVfmadd132sdVpdHsdWsdKmask
    0x280, // EvexVfmadd213ssVpsHssWssKmask
    0x280, // EvexVfmadd213sdVpdHsdWsdKmask
    0x280, // EvexVfmadd231ssVpsHssWssKmask
    0x280, // EvexVfmadd231sdVpdHsdWsdKmask
    0x080, // EvexVfmaddsub132psVpsHpsWps
    0x080, // EvexVfmaddsub132pdVpdHpdWpd
    0x080, // EvexVfmaddsub213psVpsHpsWps
    0x080, // EvexVfmaddsub213pdVpdHpdWpd
    0x080, // EvexVfmaddsub231psVpsHpsWps
    0x080, // EvexVfmaddsub231pdVpdHpdWpd
    0x080, // EvexVfmaddsub132psVpsHpsWpsKmask
    0x080, // EvexVfmaddsub132pdVpdHpdWpdKmask
    0x080, // EvexVfmaddsub213psVpsHpsWpsKmask
    0x080, // EvexVfmaddsub213pdVpdHpdWpdKmask
    0x080, // EvexVfmaddsub231psVpsHpsWpsKmask
    0x080, // EvexVfmaddsub231pdVpdHpdWpdKmask
    0x080, // EvexVfmsubadd132psVpsHpsWps
    0x080, // EvexVfmsubadd132pdVpdHpdWpd
    0x080, // EvexVfmsubadd213psVpsHpsWps
    0x080, // EvexVfmsubadd213pdVpdHpdWpd
    0x080, // EvexVfmsubadd231psVpsHpsWps
    0x080, // EvexVfmsubadd231pdVpdHpdWpd
    0x080, // EvexVfmsubadd132psVpsHpsWpsKmask
    0x080, // EvexVfmsubadd132pdVpdHpdWpdKmask
    0x080, // EvexVfmsubadd213psVpsHpsWpsKmask
    0x080, // EvexVfmsubadd213pdVpdHpdWpdKmask
    0x080, // EvexVfmsubadd231psVpsHpsWpsKmask
    0x080, // EvexVfmsubadd231pdVpdHpdWpdKmask
    0x080, // EvexVfmsub132psVpsHpsWps
    0x080, // EvexVfmsub132pdVpdHpdWpd
    0x080, // EvexVfmsub213psVpsHpsWps
    0x080, // EvexVfmsub213pdVpdHpdWpd
    0x080, // EvexVfmsub231psVpsHpsWps
    0x080, // EvexVfmsub231pdVpdHpdWpd
    0x080, // EvexVfmsub132psVpsHpsWpsKmask
    0x080, // EvexVfmsub132pdVpdHpdWpdKmask
    0x080, // EvexVfmsub213psVpsHpsWpsKmask
    0x080, // EvexVfmsub213pdVpdHpdWpdKmask
    0x080, // EvexVfmsub231psVpsHpsWpsKmask
    0x080, // EvexVfmsub231pdVpdHpdWpdKmask
    0x280, // EvexVfmsub132ssVpsHssWss
    0x280, // EvexVfmsub132sdVpdHsdWsd
    0x280, // EvexVfmsub213ssVpsHssWss
    0x280, // EvexVfmsub213sdVpdHsdWsd
    0x280, // EvexVfmsub231ssVpsHssWss
    0x280, // EvexVfmsub231sdVpdHsdWsd
    0x280, // EvexVfmsub132ssVpsHssWssKmask
    0x280, // EvexVfmsub132sdVpdHsdWsdKmask
    0x280, // EvexVfmsub213ssVpsHssWssKmask
    0x280, // EvexVfmsub213sdVpdHsdWsdKmask
    0x280, // EvexVfmsub231ssVpsHssWssKmask
    0x280, // EvexVfmsub231sdVpdHsdWsdKmask
    0x080, // EvexVfnmadd132psVpsHpsWps
    0x080, // EvexVfnmadd132pdVpdHpdWpd
    0x080, // EvexVfnmadd213psVpsHpsWps
    0x080, // EvexVfnmadd213pdVpdHpdWpd
    0x080, // EvexVfnmadd231psVpsHpsWps
    0x080, // EvexVfnmadd231pdVpdHpdWpd
    0x080, // EvexVfnmadd132psVpsHpsWpsKmask
    0x080, // EvexVfnmadd132pdVpdHpdWpdKmask
    0x080, // EvexVfnmadd213psVpsHpsWpsKmask
    0x080, // EvexVfnmadd213pdVpdHpdWpdKmask
    0x080, // EvexVfnmadd231psVpsHpsWpsKmask
    0x080, // EvexVfnmadd231pdVpdHpdWpdKmask
    0x280, // EvexVfnmadd132ssVpsHssWss
    0x280, // EvexVfnmadd132sdVpdHsdWsd
    0x280, // EvexVfnmadd213ssVpsHssWss
    0x280, // EvexVfnmadd213sdVpdHsdWsd
    0x280, // EvexVfnmadd231ssVpsHssWss
    0x280, // EvexVfnmadd231sdVpdHsdWsd
    0x280, // EvexVfnmadd132ssVpsHssWssKmask
    0x280, // EvexVfnmadd132sdVpdHsdWsdKmask
    0x280, // EvexVfnmadd213ssVpsHssWssKmask
    0x280, // EvexVfnmadd213sdVpdHsdWsdKmask
    0x280, // EvexVfnmadd231ssVpsHssWssKmask
    0x280, // EvexVfnmadd231sdVpdHsdWsdKmask
    0x080, // EvexVfnmsub132psVpsHpsWps
    0x080, // EvexVfnmsub132pdVpdHpdWpd
    0x080, // EvexVfnmsub213psVpsHpsWps
    0x080, // EvexVfnmsub213pdVpdHpdWpd
    0x080, // EvexVfnmsub231psVpsHpsWps
    0x080, // EvexVfnmsub231pdVpdHpdWpd
    0x080, // EvexVfnmsub132psVpsHpsWpsKmask
    0x080, // EvexVfnmsub132pdVpdHpdWpdKmask
    0x080, // EvexVfnmsub213psVpsHpsWpsKmask
    0x080, // EvexVfnmsub213pdVpdHpdWpdKmask
    0x080, // EvexVfnmsub231psVpsHpsWpsKmask
    0x080, // EvexVfnmsub231pdVpdHpdWpdKmask
    0x280, // EvexVfnmsub132ssVpsHssWss
    0x280, // EvexVfnmsub132sdVpdHsdWsd
    0x280, // EvexVfnmsub213ssVpsHssWss
    0x280, // EvexVfnmsub213sdVpdHsdWsd
    0x280, // EvexVfnmsub231ssVpsHssWss
    0x280, // EvexVfnmsub231sdVpdHsdWsd
    0x280, // EvexVfnmsub132ssVpsHssWssKmask
    0x280, // EvexVfnmsub132sdVpdHsdWsdKmask
    0x280, // EvexVfnmsub213ssVpsHssWssKmask
    0x280, // EvexVfnmsub213sdVpdHsdWsdKmask
    0x280, // EvexVfnmsub231ssVpsHssWssKmask
    0x280, // EvexVfnmsub231sdVpdHsdWsdKmask
    0x380, // EvexVpcmpbKgqHdqWdqIb
    0x380, // EvexVpcmpwKgdHdqWdqIb
    0x380, // EvexVpcmpubKgqHdqWdqIb
    0x380, // EvexVpcmpuwKgdHdqWdqIb
    0x180, // EvexVpcmpdKgwHdqWdqIb
    0x180, // EvexVpcmpqKgbHdqWdqIb
    0x180, // EvexVpcmpudKgwHdqWdqIb
    0x180, // EvexVpcmpuqKgbHdqWdqIb
    0x380, // EvexVptestmbKgqHdqWdq
    0x380, // EvexVptestmwKgdHdqWdq
    0x380, // EvexVptestnmbKgqHdqWdq
    0x380, // EvexVptestnmwKgdHdqWdq
    0x180, // EvexVptestmdKgwHdqWdq
    0x180, // EvexVptestmqKgbHdqWdq
    0x180, // EvexVptestnmdKgwHdqWdq
    0x180, // EvexVptestnmqKgbHdqWdq
    0x180, // EvexVpternlogdVdqHdqWdqIb
    0x180, // EvexVpternlogqVdqHdqWdqIb
    0x180, // EvexVpternlogdVdqHdqWdqIbKmask
    0x180, // EvexVpternlogqVdqHdqWdqIbKmask
    0x280, // EvexVgatherdpsVpsVsib
    0x280, // EvexVgatherdpdVpdVsib
    0x280, // EvexVgatherqpsVpsVsib
    0x280, // EvexVgatherqpdVpdVsib
    0x280, // EvexVgatherddVdqVsib
    0x280, // EvexVgatherdqVdqVsib
    0x280, // EvexVgatherqdVdqVsib
    0x280, // EvexVgatherqqVdqVsib
    0x280, // EvexVscatterdpsVsibVps
    0x280, // EvexVscatterdpdVsibVpd
    0x280, // EvexVscatterqpsVsibVps
    0x280, // EvexVscatterqpdVsibVpd
    0x280, // EvexVscatterddVsibVdq
    0x280, // EvexVscatterdqVsibVdq
    0x280, // EvexVscatterqdVsibVdq
    0x280, // EvexVscatterqqVsibVdq
    0x180, // EvexVblendmpsVpsHpsWps
    0x180, // EvexVblendmpdVpdHpdWpd
    0x180, // EvexVpblendmdVdqHdqWdq
    0x180, // EvexVpblendmqVdqHdqWdq
    0x380, // EvexVpblendmbVdqHdqWdq
    0x380, // EvexVpblendmwVdqHdqWdq
    0x180, // EvexVshufi32x4VdqHdqWdqIbKmask
    0x180, // EvexVshufi64x2VdqHdqWdqIbKmask
    0x180, // EvexVshuff32x4VpsHpsWpsIbKmask
    0x180, // EvexVshuff64x2VpdHpdWpdIbKmask
    0x380, // EvexVexpandpsVpsWps
    0x380, // EvexVexpandpdVpdWpd
    0x380, // EvexVexpandpsVpsWpsKmask
    0x380, // EvexVexpandpdVpdWpdKmask
    0x380, // EvexVcompresspsWpsVps
    0x380, // EvexVcompresspdWpdVpd
    0x380, // EvexVcompresspsWpsVpsKmask
    0x380, // EvexVcompresspdWpdVpdKmask
    0x380, // EvexVpexpandbVdqWdq
    0x380, // EvexVpexpandwVdqWdq
    0x380, // EvexVpexpandbVdqWdqKmask
    0x380, // EvexVpexpandwVdqWdqKmask
    0x380, // EvexVpexpanddVdqWdq
    0x380, // EvexVpexpandqVdqWdq
    0x380, // EvexVpexpanddVdqWdqKmask
    0x380, // EvexVpexpandqVdqWdqKmask
    0x380, // EvexVpcompressbWdqVdq
    0x380, // EvexVpcompresswWdqVdq
    0x380, // EvexVpcompressbWdqVdqKmask
    0x380, // EvexVpcompresswWdqVdqKmask
    0x380, // EvexVpcompressdWdqVdq
    0x380, // EvexVpcompressqWdqVdq
    0x380, // EvexVpcompressdWdqVdqKmask
    0x380, // EvexVpcompressqWdqVdqKmask
    0x280, // EvexVfixupimmssVssHssWssIbKmask
    0x280, // EvexVfixupimmsdVsdHsdWsdIbKmask
    0x080, // EvexVfixupimmpsVpsHpsWpsIb
    0x080, // EvexVfixupimmpdVpdHpdWpdIb
    0x080, // EvexVfixupimmpsVpsHpsWpsIbKmask
    0x080, // EvexVfixupimmpdVpdHpdWpdIbKmask
    0x180, // EvexVfpclasspsKgwWpsIbKmask
    0x180, // EvexVfpclasspdKgbWpdIbKmask
    0x380, // EvexVfpclassssKgbWssIbKmask
    0x380, // EvexVfpclasssdKgbWsdIbKmask
    0x080, // EvexVreducepsVpsWpsIbKmask
    0x080, // EvexVreducepdVpdWpdIbKmask
    0x280, // EvexVreducessVssHpsWssIbKmask
    0x280, // EvexVreducesdVsdHpdWsdIbKmask
    0x180, // EvexVpermt2dVdqHdqWdqKmask
    0x180, // EvexVpermt2qVdqHdqWdqKmask
    0x180, // EvexVpermi2dVdqHdqWdqKmask
    0x180, // EvexVpermi2qVdqHdqWdqKmask
    0x180, // EvexVpermt2psVpsHpsWpsKmask
    0x180, // EvexVpermt2pdVpdHpdWpdKmask
    0x180, // EvexVpermi2psVpsHpsWpsKmask
    0x180, // EvexVpermi2pdVpdHpdWpdKmask
    0x180, // EvexVpermdVdqHdqWdqKmask
    0x180, // EvexVpermqVdqHdqWdqKmask
    0x180, // EvexVpermpsVpsHpsWpsKmask
    0x180, // EvexVpermpdVpdHpdWpdKmask
    0x180, // EvexVpconflictdVdqWdqKmask
    0x180, // EvexVpconflictqVdqWdqKmask
    0x180, // EvexVplzcntdVdqWdqKmask
    0x180, // EvexVplzcntqVdqWdqKmask
    0x380, // EvexVpmovm2bVdqKeq
    0x380, // EvexVpmovm2wVdqKed
    0x380, // EvexVpmovm2dVdqKew
    0x380, // EvexVpmovm2qVdqKeb
    0x380, // EvexVpmovb2mKgqWdq
    0x380, // EvexVpmovw2mKgdWdq
    0x380, // EvexVpmovd2mKgwWdq
    0x380, // EvexVpmovq2mKgbWdq
    0x380, // EvexVpopcntbVdqWdqKmask
    0x380, // EvexVpopcntwVdqWdqKmask
    0x180, // EvexVpopcntdVdqWdqKmask
    0x180, // EvexVpopcntqVdqWdqKmask
    0x180, // EvexVpshrddVdqHdqWdqIbKmask
    0x180, // EvexVpshrdqVdqHdqWdqIbKmask
    0x180, // EvexVpshrdvdVdqHdqWdqKmask
    0x180, // EvexVpshrdvqVdqHdqWdqKmask
    0x180, // EvexVpshlddVdqHdqWdqIbKmask
    0x180, // EvexVpshldqVdqHdqWdqIbKmask
    0x180, // EvexVpshldvdVdqHdqWdqKmask
    0x180, // EvexVpshldvqVdqHdqWdqKmask
    0x280, // EvexVcvtss2siGdWss
    0x280, // EvexVcvtss2siGqWss
    0x280, // EvexVcvtsd2siGdWsd
    0x280, // EvexVcvtsd2siGqWsd
    0x280, // EvexVcvttss2siGdWss
    0x280, // EvexVcvttss2siGqWss
    0x280, // EvexVcvttsd2siGdWsd
    0x280, // EvexVcvttsd2siGqWsd
    0x380, // EvexVmovdVdqEd
    0x380, // EvexVmovqVdqEq
    0x380, // EvexVmovdEdVd
    0x380, // EvexVmovqEqVq
    0x280, // EvexVcvtsi2ssVssEd
    0x280, // EvexVcvtsi2ssVssEq
    0x280, // EvexVcvtsi2sdVsdEd
    0x280, // EvexVcvtsi2sdVsdEq
    0x280, // EvexVcvtusi2ssVssEd
    0x280, // EvexVcvtusi2ssVssEq
    0x280, // EvexVcvtusi2sdVsdEd
    0x280, // EvexVcvtusi2sdVsdEq
    0x280, // EvexVcvtss2usiGdWss
    0x280, // EvexVcvtss2usiGqWss
    0x280, // EvexVcvtsd2usiGdWsd
    0x280, // EvexVcvtsd2usiGqWsd
    0x280, // EvexVcvttss2usiGdWss
    0x280, // EvexVcvttss2usiGqWss
    0x280, // EvexVcvttsd2usiGdWsd
    0x280, // EvexVcvttsd2usiGqWsd
    0x380, // EvexVpinsrbVdqEbIb
    0x380, // EvexVpinsrwVdqEwIb
    0x380, // EvexVpextrwGdUdqIb
    0x380, // EvexVpextrbEdVdqIbR
    0x380, // EvexVpextrbMbVdqIbM
    0x380, // EvexVpextrwEdVdqIbR
    0x380, // EvexVpextrwMwVdqIbM
    0x380, // EvexVpinsrdVdqEdIb
    0x380, // EvexVpinsrqVdqEqIb
    0x380, // EvexVpextrdEdVdqIb
    0x380, // EvexVpextrqEqVdqIb
    0x380, // EvexVpbroadcastmb2qVdqKeb
    0x380, // EvexVpbroadcastmw2dVdqKew
    0x180, // EvexVpdpbusdVdqHdqWdq
    0x180, // EvexVpdpbusdsVdqHdqWdq
    0x180, // EvexVpdpwssdVdqHdqWdq
    0x180, // EvexVpdpwssdsVdqHdqWdq
    0x180, // EvexVpdpbusdVdqHdqWdqKmask
    0x180, // EvexVpdpbusdsVdqHdqWdqKmask
    0x180, // EvexVpdpwssdVdqHdqWdqKmask
    0x180, // EvexVpdpwssdsVdqHdqWdqKmask
    0x380, // EvexVpshufbitqmbKgqHdqWdqKmask
    0x180, // EvexVp2intersectdKgqHdqWdq
    0x180, // EvexVp2intersectqKgqHdqWdq
    0x380, // EvexVpshrdwVdqHdqWdqIbKmask
    0x380, // EvexVpshrdvwVdqHdqWdqKmask
    0x380, // EvexVpshldwVdqHdqWdqIbKmask
    0x380, // EvexVpshldvwVdqHdqWdqKmask
    0x280, // EvexVaddshVshHphWsh
    0x280, // EvexVaddshVshHphWshKmask
    0x280, // EvexVsubshVshHphWsh
    0x280, // EvexVsubshVshHphWshKmask
    0x280, // EvexVmulshVshHphWsh
    0x280, // EvexVmulshVshHphWshKmask
    0x280, // EvexVdivshVshHphWsh
    0x280, // EvexVdivshVshHphWshKmask
    0x280, // EvexVminshVshHphWsh
    0x280, // EvexVminshVshHphWshKmask
    0x280, // EvexVmaxshVshHphWsh
    0x280, // EvexVmaxshVshHphWshKmask
    0x280, // EvexVscalefshVshHphWsh
    0x280, // EvexVscalefshVshHphWshKmask
    0x080, // EvexVaddphVphHphWph
    0x080, // EvexVaddphVphHphWphKmask
    0x080, // EvexVsubphVphHphWph
    0x080, // EvexVsubphVphHphWphKmask
    0x080, // EvexVmulphVphHphWph
    0x080, // EvexVmulphVphHphWphKmask
    0x080, // EvexVdivphVphHphWph
    0x080, // EvexVdivphVphHphWphKmask
    0x080, // EvexVminphVphHphWph
    0x080, // EvexVminphVphHphWphKmask
    0x080, // EvexVmaxphVphHphWph
    0x080, // EvexVmaxphVphHphWphKmask
    0x080, // EvexVscalefphVphHphWph
    0x080, // EvexVscalefphVphHphWphKmask
    0x280, // EvexVfmadd132shVphHshWsh
    0x280, // EvexVfmadd132shVphHshWshKmask
    0x280, // EvexVfmadd213shVphHshWsh
    0x280, // EvexVfmadd213shVphHshWshKmask
    0x280, // EvexVfmadd231shVphHshWsh
    0x280, // EvexVfmadd231shVphHshWshKmask
    0x280, // EvexVfnmadd132shVphHshWsh
    0x280, // EvexVfnmadd132shVphHshWshKmask
    0x280, // EvexVfnmadd213shVphHshWsh
    0x280, // EvexVfnmadd213shVphHshWshKmask
    0x280, // EvexVfnmadd231shVphHshWsh
    0x280, // EvexVfnmadd231shVphHshWshKmask
    0x280, // EvexVfmsub132shVphHshWsh
    0x280, // EvexVfmsub132shVphHshWshKmask
    0x280, // EvexVfmsub213shVphHshWsh
    0x280, // EvexVfmsub213shVphHshWshKmask
    0x280, // EvexVfmsub231shVphHshWsh
    0x280, // EvexVfmsub231shVphHshWshKmask
    0x280, // EvexVfnmsub132shVphHshWsh
    0x280, // EvexVfnmsub132shVphHshWshKmask
    0x280, // EvexVfnmsub213shVphHshWsh
    0x280, // EvexVfnmsub213shVphHshWshKmask
    0x280, // EvexVfnmsub231shVphHshWsh
    0x280, // EvexVfnmsub231shVphHshWshKmask
    0x080, // EvexVfmadd132phVphHphWph
    0x080, // EvexVfmadd132phVphHphWphKmask
    0x080, // EvexVfmadd213phVphHphWph
    0x080, // EvexVfmadd213phVphHphWphKmask
    0x080, // EvexVfmadd231phVphHphWph
    0x080, // EvexVfmadd231phVphHphWphKmask
    0x080, // EvexVfnmadd132phVphHphWph
    0x080, // EvexVfnmadd132phVphHphWphKmask
    0x080, // EvexVfnmadd213phVphHphWph
    0x080, // EvexVfnmadd213phVphHphWphKmask
    0x080, // EvexVfnmadd231phVphHphWph
    0x080, // EvexVfnmadd231phVphHphWphKmask
    0x080, // EvexVfmsub132phVphHphWph
    0x080, // EvexVfmsub132phVphHphWphKmask
    0x080, // EvexVfmsub213phVphHphWph
    0x080, // EvexVfmsub213phVphHphWphKmask
    0x080, // EvexVfmsub231phVphHphWph
    0x080, // EvexVfmsub231phVphHphWphKmask
    0x080, // EvexVfnmsub132phVphHphWph
    0x080, // EvexVfnmsub132phVphHphWphKmask
    0x080, // EvexVfnmsub213phVphHphWph
    0x080, // EvexVfnmsub213phVphHphWphKmask
    0x080, // EvexVfnmsub231phVphHphWph
    0x080, // EvexVfnmsub231phVphHphWphKmask
    0x080, // EvexVfmaddsub132phVphHphWph
    0x080, // EvexVfmaddsub132phVphHphWphKmask
    0x080, // EvexVfmaddsub213phVphHphWph
    0x080, // EvexVfmaddsub213phVphHphWphKmask
    0x080, // EvexVfmaddsub231phVphHphWph
    0x080, // EvexVfmaddsub231phVphHphWphKmask
    0x080, // EvexVfmsubadd132phVphHphWph
    0x080, // EvexVfmsubadd132phVphHphWphKmask
    0x080, // EvexVfmsubadd213phVphHphWph
    0x080, // EvexVfmsubadd213phVphHphWphKmask
    0x080, // EvexVfmsubadd231phVphHphWph
    0x080, // EvexVfmsubadd231phVphHphWphKmask
    0x180, // EvexVfpclassphKgdWphIbKmask
    0x380, // EvexVfpclassshKgbWshIbKmask
    0x280, // EvexVucomishVshWsh
    0x280, // EvexVcomishVshWsh
    0x080, // EvexVcmpphKgdHphWphIb
    0x280, // EvexVcmpshKgbHshWshIb
    0x080, // EvexVsqrtphVphWph
    0x080, // EvexVsqrtphVphWphKmask
    0x280, // EvexVsqrtshVshHphWsh
    0x280, // EvexVsqrtshVshHphWshKmask
    0x080, // EvexVgetexpphVphWph
    0x080, // EvexVgetexpphVphWphKmask
    0x280, // EvexVgetexpshVshHphWsh
    0x280, // EvexVgetexpshVshHphWshKmask
    0x280, // EvexVmovshVshWsh
    0x280, // EvexVmovshWshVsh
    0x280, // EvexVmovshVshWshKmask
    0x280, // EvexVmovshWshVshKmask
    0x180, // EvexVmovshVshHphWsh
    0x180, // EvexVmovshWshHphVsh
    0x180, // EvexVmovshVshHphWshKmask
    0x180, // EvexVmovshWshHphVshKmask
    0x280, // EvexVmovwVshEw
    0x280, // EvexVmovwEdVsh
    0x080, // EvexVcvtph2uwVdqWps
    0x080, // EvexVcvtph2uwVdqWpsKmask
    0x080, // EvexVcvtph2wVdqWps
    0x080, // EvexVcvtph2wVdqWpsKmask
    0x080, // EvexVcvttph2uwVdqWps
    0x080, // EvexVcvttph2uwVdqWpsKmask
    0x080, // EvexVcvttph2wVdqWps
    0x080, // EvexVcvttph2wVdqWpsKmask
    0x080, // EvexVcvtuw2phVphWdq
    0x080, // EvexVcvtuw2phVphWdqKmask
    0x080, // EvexVcvtw2phVphWdq
    0x080, // EvexVcvtw2phVphWdqKmask
    0x080, // EvexVcvtph2psxVpsWph
    0x080, // EvexVcvtph2psxVpsWphKmask
    0x080, // EvexVcvtph2dqVdqWph
    0x080, // EvexVcvtph2dqVdqWphKmask
    0x080, // EvexVcvtph2udqVdqWph
    0x080, // EvexVcvtph2udqVdqWphKmask
    0x080, // EvexVcvttph2dqVdqWph
    0x080, // EvexVcvttph2dqVdqWphKmask
    0x080, // EvexVcvttph2udqVdqWph
    0x080, // EvexVcvttph2udqVdqWphKmask
    0x080, // EvexVcvtph2pdVpdWph
    0x080, // EvexVcvtph2pdVpdWphKmask
    0x080, // EvexVcvtph2qqVdqWph
    0x080, // EvexVcvtph2qqVdqWphKmask
    0x080, // EvexVcvtph2uqqVdqWph
    0x080, // EvexVcvtph2uqqVdqWphKmask
    0x080, // EvexVcvttph2qqVdqWph
    0x080, // EvexVcvttph2qqVdqWphKmask
    0x080, // EvexVcvttph2uqqVdqWph
    0x080, // EvexVcvttph2uqqVdqWphKmask
    0x080, // EvexVcvtps2phxVphWdq
    0x080, // EvexVcvtps2phxVphWdqKmask
    0x080, // EvexVcvtdq2phVphWdq
    0x080, // EvexVcvtdq2phVphWdqKmask
    0x080, // EvexVcvtudq2phVphWdq
    0x080, // EvexVcvtudq2phVphWdqKmask
    0x080, // EvexVcvtpd2phVphWdq
    0x080, // EvexVcvtpd2phVphWdqKmask
    0x080, // EvexVcvtqq2phVphWdq
    0x080, // EvexVcvtqq2phVphWdqKmask
    0x080, // EvexVcvtuqq2phVphWdq
    0x080, // EvexVcvtuqq2phVphWdqKmask
    0x280, // EvexVcvtsh2ssVssWsh
    0x280, // EvexVcvtsh2ssVssWshKmask
    0x280, // EvexVcvtsh2sdVsdWsh
    0x280, // EvexVcvtsh2sdVsdWshKmask
    0x280, // EvexVcvtss2shVssWsh
    0x280, // EvexVcvtss2shVssWshKmask
    0x280, // EvexVcvtsd2shVssWsh
    0x280, // EvexVcvtsd2shVssWshKmask
    0x280, // EvexVcvtsh2siGdWss
    0x280, // EvexVcvtsh2siGqWss
    0x280, // EvexVcvtsh2usiGdWss
    0x280, // EvexVcvtsh2usiGqWss
    0x280, // EvexVcvttsh2siGdWss
    0x280, // EvexVcvttsh2siGqWss
    0x280, // EvexVcvttsh2usiGdWss
    0x280, // EvexVcvttsh2usiGqWss
    0x280, // EvexVcvtsi2shVshEd
    0x280, // EvexVcvtsi2shVshEq
    0x280, // EvexVcvtusi2shVshEd
    0x280, // EvexVcvtusi2shVshEq
    0x080, // EvexVgetmantphVphWphIbKmask
    0x280, // EvexVgetmantshVshHphWshIbKmask
    0x080, // EvexVreducephVphWphIbKmask
    0x280, // EvexVreduceshVshHphWshIbKmask
    0x080, // EvexVrndscalephVphWphIbKmask
    0x280, // EvexVrndscaleshVshHphWshIbKmask
    0x180, // EvexVrcpphVphWphKmask
    0x380, // EvexVrcpshVshHphWshKmask
    0x180, // EvexVrsqrtphVphWphKmask
    0x380, // EvexVrsqrtshVshHphWshKmask
    0x280, // EvexVfmulcshVshHphWshKmask
    0x280, // EvexVfcmulcshVshHphWshKmask
    0x080, // EvexVfmulcphVphHphWphKmask
    0x080, // EvexVfcmulcphVphHphWphKmask
    0x280, // EvexVfmaddcshVshHphWshKmask
    0x280, // EvexVfcmaddcshVshHphWshKmask
    0x080, // EvexVfmaddcphVphHphWphKmask
    0x080, // EvexVfcmaddcphVphHphWphKmask
    0x380, // EvexVaesencVdqHdqWdq
    0x380, // EvexVaesenclastVdqHdqWdq
    0x380, // EvexVaesdecVdqHdqWdq
    0x380, // EvexVaesdeclastVdqHdqWdq
    0x380, // EvexVpclmulqdqVdqHdqWdqIb
    0x180, // EvexVgf2p8affineqbVdqHdqWdqIbKmask
    0x180, // EvexVgf2p8affineinvqbVdqHdqWdqIbKmask
    0x380, // EvexVgf2p8mulbVdqHdqWdqKmask
    0x380, // EvexVsm4key4VdqHdqWdq
    0x380, // EvexVsm4rnds4VdqHdqWdq
    0x280, // EvexVucomxssVssWss
    0x280, // EvexVcomxssVssWss
    0x280, // EvexVucomxsdVsdWsd
    0x280, // EvexVcomxsdVsdWsd
    0x280, // EvexVucomxshVshWsh
    0x280, // EvexVcomxshVshWsh
    0x180, // EvexVpdpbssdVdqHdqWdq
    0x180, // EvexVpdpbssdsVdqHdqWdq
    0x180, // EvexVpdpbsudVdqHdqWdq
    0x180, // EvexVpdpbsudsVdqHdqWdq
    0x180, // EvexVpdpbuudVdqHdqWdq
    0x180, // EvexVpdpbuudsVdqHdqWdq
    0x180, // EvexVpdpbssdVdqHdqWdqKmask
    0x180, // EvexVpdpbssdsVdqHdqWdqKmask
    0x180, // EvexVpdpbsudVdqHdqWdqKmask
    0x180, // EvexVpdpbsudsVdqHdqWdqKmask
    0x180, // EvexVpdpbuudVdqHdqWdqKmask
    0x180, // EvexVpdpbuudsVdqHdqWdqKmask
    0x180, // EvexVpdpwsudVdqHdqWdq
    0x180, // EvexVpdpwsudsVdqHdqWdq
    0x180, // EvexVpdpwusdVdqHdqWdq
    0x180, // EvexVpdpwusdsVdqHdqWdq
    0x180, // EvexVpdpwuudVdqHdqWdq
    0x180, // EvexVpdpwuudsVdqHdqWdq
    0x180, // EvexVpdpwsudVdqHdqWdqKmask
    0x180, // EvexVpdpwsudsVdqHdqWdqKmask
    0x180, // EvexVpdpwusdVdqHdqWdqKmask
    0x180, // EvexVpdpwusdsVdqHdqWdqKmask
    0x180, // EvexVpdpwuudVdqHdqWdqKmask
    0x180, // EvexVpdpwuudsVdqHdqWdqKmask
    0x380, // EvexVmpsadbwVdqHdqWdqIb
    0x380, // EvexVmpsadbwVdqHdqWdqIbKmask
    0x180, // EvexVdpphpsVpsHdqWdqKmask
    0x180, // EvexVaddbf16VphHphWph
    0x180, // EvexVaddbf16VphHphWphKmask
    0x180, // EvexVsubbf16VphHphWph
    0x180, // EvexVsubbf16VphHphWphKmask
    0x180, // EvexVdivbf16VphHphWph
    0x180, // EvexVdivbf16VphHphWphKmask
    0x180, // EvexVmulbf16VphHphWph
    0x180, // EvexVmulbf16VphHphWphKmask
    0x000, // EvexVminpbf16VphHphWph
    0x000, // EvexVminpbf16VphHphWphKmask
    0x000, // EvexVmaxpbf16VphHphWph
    0x000, // EvexVmaxpbf16VphHphWphKmask
    0x180, // EvexVscalefpbf16VphHphWph
    0x180, // EvexVscalefpbf16VphHphWphKmask
    0x180, // EvexVsqrtbf16VphWph
    0x180, // EvexVsqrtbf16VphWphKmask
    0x180, // EvexVgetexppbf16VphWph
    0x180, // EvexVgetexppbf16VphWphKmask
    0x180, // EvexVfmadd132bf16VphHphWph
    0x180, // EvexVfmadd132bf16VphHphWphKmask
    0x180, // EvexVfmadd213bf16VphHphWph
    0x180, // EvexVfmadd213bf16VphHphWphKmask
    0x180, // EvexVfmadd231bf16VphHphWph
    0x180, // EvexVfmadd231bf16VphHphWphKmask
    0x180, // EvexVfmsub132bf16VphHphWph
    0x180, // EvexVfmsub132bf16VphHphWphKmask
    0x180, // EvexVfmsub213bf16VphHphWph
    0x180, // EvexVfmsub213bf16VphHphWphKmask
    0x180, // EvexVfmsub231bf16VphHphWph
    0x180, // EvexVfmsub231bf16VphHphWphKmask
    0x180, // EvexVfnmadd132bf16VphHphWph
    0x180, // EvexVfnmadd132bf16VphHphWphKmask
    0x180, // EvexVfnmadd213bf16VphHphWph
    0x180, // EvexVfnmadd213bf16VphHphWphKmask
    0x180, // EvexVfnmadd231bf16VphHphWph
    0x180, // EvexVfnmadd231bf16VphHphWphKmask
    0x180, // EvexVfnmsub132bf16VphHphWph
    0x180, // EvexVfnmsub132bf16VphHphWphKmask
    0x180, // EvexVfnmsub213bf16VphHphWph
    0x180, // EvexVfnmsub213bf16VphHphWphKmask
    0x180, // EvexVfnmsub231bf16VphHphWph
    0x180, // EvexVfnmsub231bf16VphHphWphKmask
    0x180, // EvexVfpclasspbf16KgdWphIbKmask
    0x180, // EvexVcmppbf16KgdHphWphIb
    0x380, // EvexVcomisbf16VshWsh
    0x180, // EvexVgetmantpbf16VphWphIbKmask
    0x180, // EvexVreducebf16VphWphIbKmask
    0x180, // EvexVrndscalebf16VphWphIbKmask
    0x180, // EvexVrcppbf16VphWph
    0x180, // EvexVrcppbf16VphWphKmask
    0x180, // EvexVrsqrtpbf16VphWph
    0x180, // EvexVrsqrtpbf16VphWphKmask
    0x080, // EvexVminmaxpsVpsHpsWpsIbKmask
    0x280, // EvexVminmaxssVssHpsWssIbKmask
    0x080, // EvexVminmaxpdVpdHpdWpdIbKmask
    0x280, // EvexVminmaxsdVsdHpdWsdIbKmask
    0x080, // EvexVminmaxphVphHphWphIbKmask
    0x280, // EvexVminmaxshVshHphWshIbKmask
    0x180, // EvexVminmaxbf16VphHphWphIbKmask
    0x080, // EvexVcvt2ps2phxVphHpsWpsKmask
    0x080, // EvexVcvttps2qqsVdqWps
    0x080, // EvexVcvttps2qqsVdqWpsKmask
    0x080, // EvexVcvttpd2qqsVdqWpd
    0x080, // EvexVcvttpd2qqsVdqWpdKmask
    0x080, // EvexVcvttps2uqqsVdqWps
    0x080, // EvexVcvttps2uqqsVdqWpsKmask
    0x080, // EvexVcvttpd2uqqsVdqWpd
    0x080, // EvexVcvttpd2uqqsVdqWpdKmask
    0x080, // EvexVcvttps2dqsVdqWps
    0x080, // EvexVcvttps2dqsVdqWpsKmask
    0x080, // EvexVcvttpd2dqsVdqWpd
    0x080, // EvexVcvttpd2dqsVdqWpdKmask
    0x080, // EvexVcvttps2udqsVdqWps
    0x080, // EvexVcvttpd2udqsVdqWpd
    0x080, // EvexVcvttps2udqsVdqWpsKmask
    0x080, // EvexVcvttpd2udqsVdqWpdKmask
    0x280, // EvexVcvttss2sisGdWss
    0x280, // EvexVcvttss2sisGqWss
    0x280, // EvexVcvttsd2sisGdWsd
    0x280, // EvexVcvttsd2sisGqWsd
    0x280, // EvexVcvttss2usisGdWss
    0x280, // EvexVcvttss2usisGqWss
    0x280, // EvexVcvttsd2usisGdWsd
    0x280, // EvexVcvttsd2usisGqWsd
    0x380, // EvexVmovwVshWsh
    0x380, // EvexVmovwWshVsh
    0x380, // EvexVmovdVdWd
    0x380, // EvexVmovdWdVd
    0x380, // EvexVcvthf82phVphWf8Kmask
    0x180, // EvexVcvtph2bf8Vf8hdqWphKmask
    0x180, // EvexVcvtph2bf8sVf8hdqWphKmask
    0x180, // EvexVcvt2ph2bf8Vf8hdqWphKmask
    0x180, // EvexVcvt2ph2bf8sVf8hdqWphKmask
    0x180, // EvexVcvtbiasph2bf8Vf8hdqWphKmask
    0x180, // EvexVcvtbiasph2bf8sVf8hdqWphKmask
    0x180, // EvexVcvtph2hf8Vf8hdqWphKmask
    0x180, // EvexVcvtph2hf8sVf8hdqWphKmask
    0x180, // EvexVcvt2ph2hf8Vf8hdqWphKmask
    0x180, // EvexVcvt2ph2hf8sVf8hdqWphKmask
    0x180, // EvexVcvtbiasph2hf8Vf8hdqWphKmask
    0x180, // EvexVcvtbiasph2hf8sVf8hdqWphKmask
    0x180, // EvexVcvtbf162ibsV8bWph
    0x180, // EvexVcvtbf162ibsV8bWphKmask
    0x180, // EvexVcvtbf162iubsV8bWph
    0x180, // EvexVcvtbf162iubsV8bWphKmask
    0x180, // EvexVcvttbf162ibsV8bWph
    0x180, // EvexVcvttbf162ibsV8bWphKmask
    0x180, // EvexVcvttbf162iubsV8bWph
    0x180, // EvexVcvttbf162iubsV8bWphKmask
    0x080, // EvexVcvtph2ibsV8bWph
    0x080, // EvexVcvtph2ibsV8bWphKmask
    0x080, // EvexVcvtph2iubsV8bWph
    0x080, // EvexVcvtph2iubsV8bWphKmask
    0x080, // EvexVcvttph2ibsV8bWph
    0x080, // EvexVcvttph2ibsV8bWphKmask
    0x080, // EvexVcvttph2iubsV8bWph
    0x080, // EvexVcvttph2iubsV8bWphKmask
    0x080, // EvexVcvtps2ibsV8bWps
    0x080, // EvexVcvtps2ibsV8bWpsKmask
    0x080, // EvexVcvtps2iubsV8bWps
    0x080, // EvexVcvtps2iubsV8bWpsKmask
    0x080, // EvexVcvttps2ibsV8bWps
    0x080, // EvexVcvttps2ibsV8bWpsKmask
    0x080, // EvexVcvttps2iubsV8bWps
    0x080, // EvexVcvttps2iubsV8bWpsKmask
    0x180, // EvexTilemovrowVdqTrmIb
    0x180, // EvexTilemovrowVdqTrmBd
    0x180, // EvexTcvtrowd2psVpsTrmIb
    0x180, // EvexTcvtrowd2psVpsTrmBd
    0x180, // EvexTcvtrowps2phlVphTrmIb
    0x180, // EvexTcvtrowps2phlVphTrmBd
    0x180, // EvexTcvtrowps2phhVphTrmIb
    0x180, // EvexTcvtrowps2phhVphTrmBd
    0x180, // EvexTcvtrowps2bf16lVphTrmIb
    0x180, // EvexTcvtrowps2bf16lVphTrmBd
    0x180, // EvexTcvtrowps2bf16hVphTrmIb
    0x180, // EvexTcvtrowps2bf16hVphTrmBd
    0x380, // EvexVmovrsbVdqWdq
    0x380, // EvexVmovrsbVdqWdqKmask
    0x380, // EvexVmovrswVdqWdq
    0x380, // EvexVmovrswVdqWdqKmask
    0x380, // EvexVmovrsdVdqWdq
    0x380, // EvexVmovrsdVdqWdqKmask
    0x380, // EvexVmovrsqVdqWdq
    0x380, // EvexVmovrsqVdqWdqKmask
    0x000, // NoAvxState
    0x000, // NoEvexState
];

/// EVEX prepare attributes for `opcode` (0 when it has none).
#[inline]
pub const fn opcode_evex_flags(opcode: Opcode) -> u16 {
    OPCODE_EVEX_FLAGS[opcode as usize]
}

/// Number of opcodes carrying EVEX prepare attributes, pinned by tests.
pub const EVEX_FLAGGED_OPCODE_COUNT: usize = 1328;

/// The CPU state an instruction needs enabled before it may execute —
/// the `BX_PREPARE_*` attribute of Bochs `bx_define_opcode`.
///
/// Bochs consults it in `assignHandler` and swaps the handler for
/// `BxNoFPU` / `BxNoMMX` / `BxNoSSE` / `BxNoAVX` / `BxNoEVEX` when the
/// state is unavailable; rusty_box applies it at icache fill, so the
/// dispatch loop pays nothing and no individual handler can forget it.
///
/// Exactly one applies per opcode. This is an enum rather than a set of
/// integer constants so that a `match` over it is exhaustive: adding a
/// class breaks every consumer at compile time instead of silently
/// falling through a catch-all arm and leaving instructions ungated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum CpuState {
    /// Base ISA — no state beyond an ordinary integer instruction.
    Base,
    /// x87 state (CR0.EM, CR0.TS).
    Fpu,
    /// MMX state.
    Mmx,
    /// SSE state (CR0.EM, CR4.OSFXSR, CR0.TS).
    Sse,
    /// AVX state (protected mode, CR4.OSXSAVE, XCR0.SSE|YMM, CR0.TS).
    Avx,
    /// AVX-512 state (AVX plus XCR0.OPMASK|ZMM_HI256|HI_ZMM).
    Evex,
    /// AMX tile state.
    Amx,
}

/// CPU state each opcode requires, from field 10 of `bx_define_opcode`.
// A `const` for the same reason as OPCODE_EVEX_FLAGS.
pub const OPCODE_STATE: [CpuState; 3679] = [
    CpuState::Base, // IaError
    CpuState::Base, // InsertedOpcode
    CpuState::Base, // Aaa
    CpuState::Base, // Aad
    CpuState::Base, // Aam
    CpuState::Base, // Aas
    CpuState::Base, // Daa
    CpuState::Base, // Das
    CpuState::Base, // AdcEbGb
    CpuState::Base, // AndEbGb
    CpuState::Base, // AddEbGb
    CpuState::Base, // CmpEbGb
    CpuState::Base, // OrEbGb
    CpuState::Base, // SbbEbGb
    CpuState::Base, // SubEbGb
    CpuState::Base, // TestEbGb
    CpuState::Base, // XorEbGb
    CpuState::Base, // AdcEwGw
    CpuState::Base, // AddEwGw
    CpuState::Base, // AndEwGw
    CpuState::Base, // CmpEwGw
    CpuState::Base, // OrEwGw
    CpuState::Base, // SbbEwGw
    CpuState::Base, // SubEwGw
    CpuState::Base, // TestEwGw
    CpuState::Base, // XorEwGw
    CpuState::Base, // AdcEdGd
    CpuState::Base, // AddEdGd
    CpuState::Base, // AndEdGd
    CpuState::Base, // CmpEdGd
    CpuState::Base, // OrEdGd
    CpuState::Base, // SbbEdGd
    CpuState::Base, // SubEdGd
    CpuState::Base, // TestEdGd
    CpuState::Base, // XorEdGd
    CpuState::Base, // AdcAlib
    CpuState::Base, // AddAlib
    CpuState::Base, // AndAlib
    CpuState::Base, // CmpAlib
    CpuState::Base, // OrAlib
    CpuState::Base, // SbbAlib
    CpuState::Base, // SubAlib
    CpuState::Base, // TestAlib
    CpuState::Base, // XorAlib
    CpuState::Base, // AdcAxiw
    CpuState::Base, // AddAxiw
    CpuState::Base, // AndAxiw
    CpuState::Base, // CmpAxiw
    CpuState::Base, // OrAxiw
    CpuState::Base, // SbbAxiw
    CpuState::Base, // SubAxiw
    CpuState::Base, // TestAxiw
    CpuState::Base, // XorAxiw
    CpuState::Base, // AdcEaxid
    CpuState::Base, // AddEaxid
    CpuState::Base, // AndEaxid
    CpuState::Base, // CmpEaxid
    CpuState::Base, // OrEaxid
    CpuState::Base, // SbbEaxid
    CpuState::Base, // SubEaxid
    CpuState::Base, // TestEaxid
    CpuState::Base, // XorEaxid
    CpuState::Base, // AddEbIb
    CpuState::Base, // OrEbIb
    CpuState::Base, // AdcEbIb
    CpuState::Base, // SbbEbIb
    CpuState::Base, // AndEbIb
    CpuState::Base, // SubEbIb
    CpuState::Base, // XorEbIb
    CpuState::Base, // TestEbIb
    CpuState::Base, // CmpEbIb
    CpuState::Base, // AddEwIw
    CpuState::Base, // OrEwIw
    CpuState::Base, // AdcEwIw
    CpuState::Base, // SbbEwIw
    CpuState::Base, // AndEwIw
    CpuState::Base, // SubEwIw
    CpuState::Base, // XorEwIw
    CpuState::Base, // TestEwIw
    CpuState::Base, // CmpEwIw
    CpuState::Base, // AddEwsIb
    CpuState::Base, // OrEwsIb
    CpuState::Base, // AdcEwsIb
    CpuState::Base, // SbbEwsIb
    CpuState::Base, // AndEwsIb
    CpuState::Base, // SubEwsIb
    CpuState::Base, // XorEwsIb
    CpuState::Base, // TestEwsIb
    CpuState::Base, // CmpEwsIb
    CpuState::Base, // AddEdId
    CpuState::Base, // OrEdId
    CpuState::Base, // AdcEdId
    CpuState::Base, // SbbEdId
    CpuState::Base, // AndEdId
    CpuState::Base, // SubEdId
    CpuState::Base, // XorEdId
    CpuState::Base, // TestEdId
    CpuState::Base, // CmpEdId
    CpuState::Base, // AddEdsIb
    CpuState::Base, // OrEdsIb
    CpuState::Base, // AdcEdsIb
    CpuState::Base, // SbbEdsIb
    CpuState::Base, // AndEdsIb
    CpuState::Base, // SubEdsIb
    CpuState::Base, // XorEdsIb
    CpuState::Base, // TestEdsIb
    CpuState::Base, // CmpEdsIb
    CpuState::Base, // XorEwGwZeroIdiom
    CpuState::Base, // XorGwEwZeroIdiom
    CpuState::Base, // XorEdGdZeroIdiom
    CpuState::Base, // XorGdEdZeroIdiom
    CpuState::Base, // SubEwGwZeroIdiom
    CpuState::Base, // SubGwEwZeroIdiom
    CpuState::Base, // SubEdGdZeroIdiom
    CpuState::Base, // SubGdEdZeroIdiom
    CpuState::Base, // AddGbEb
    CpuState::Base, // OrGbEb
    CpuState::Base, // AdcGbEb
    CpuState::Base, // SbbGbEb
    CpuState::Base, // AndGbEb
    CpuState::Base, // SubGbEb
    CpuState::Base, // XorGbEb
    CpuState::Base, // CmpGbEb
    CpuState::Base, // AdcGwEw
    CpuState::Base, // AddGwEw
    CpuState::Base, // AndGwEw
    CpuState::Base, // CmpGwEw
    CpuState::Base, // OrGwEw
    CpuState::Base, // SbbGwEw
    CpuState::Base, // SubGwEw
    CpuState::Base, // XorGwEw
    CpuState::Base, // AdcGdEd
    CpuState::Base, // AddGdEd
    CpuState::Base, // AndGdEd
    CpuState::Base, // CmpGdEd
    CpuState::Base, // OrGdEd
    CpuState::Base, // SbbGdEd
    CpuState::Base, // SubGdEd
    CpuState::Base, // XorGdEd
    CpuState::Base, // IncEb
    CpuState::Base, // IncEw
    CpuState::Base, // IncEd
    CpuState::Base, // DecEb
    CpuState::Base, // DecEw
    CpuState::Base, // DecEd
    CpuState::Base, // BsfGwEw
    CpuState::Base, // BsrGwEw
    CpuState::Base, // BsfGdEd
    CpuState::Base, // BsrGdEd
    CpuState::Base, // BtcEwGw
    CpuState::Base, // BtrEwGw
    CpuState::Base, // BtsEwGw
    CpuState::Base, // BtcEdGd
    CpuState::Base, // BtrEdGd
    CpuState::Base, // BtsEdGd
    CpuState::Base, // BtcEwIb
    CpuState::Base, // BtrEwIb
    CpuState::Base, // BtsEwIb
    CpuState::Base, // BtcEdIb
    CpuState::Base, // BtrEdIb
    CpuState::Base, // BtsEdIb
    CpuState::Base, // BtEwIb
    CpuState::Base, // BtEdIb
    CpuState::Base, // BtEwGw
    CpuState::Base, // BtEdGd
    CpuState::Base, // BoundGwMa
    CpuState::Base, // BoundGdMa
    CpuState::Base, // ArplEwGw
    CpuState::Base, // CallEd
    CpuState::Base, // CallEw
    CpuState::Base, // CallJd
    CpuState::Base, // CallJw
    CpuState::Base, // CallfOp16Ap
    CpuState::Base, // CallfOp32Ap
    CpuState::Base, // CallfOp16Ep
    CpuState::Base, // CallfOp32Ep
    CpuState::Base, // Cbw
    CpuState::Base, // Cdq
    CpuState::Base, // Cwd
    CpuState::Base, // Cwde
    CpuState::Base, // Clc
    CpuState::Base, // Cld
    CpuState::Base, // Cli
    CpuState::Base, // Clts
    CpuState::Base, // Cmc
    CpuState::Base, // Hlt
    CpuState::Base, // Clflush
    CpuState::Base, // Clflushopt
    CpuState::Base, // Clwb
    CpuState::Base, // Clzero
    CpuState::Base, // EnterOp16IwIb
    CpuState::Base, // EnterOp32IwIb
    CpuState::Base, // LeaveOp16
    CpuState::Base, // LeaveOp32
    CpuState::Base, // ImulGdEd
    CpuState::Base, // ImulGdEdId
    CpuState::Base, // ImulGdEdsIb
    CpuState::Base, // ImulGwEw
    CpuState::Base, // ImulGwEwIw
    CpuState::Base, // ImulGwEwsIb
    CpuState::Base, // InAlDx
    CpuState::Base, // InAlib
    CpuState::Base, // InAxDx
    CpuState::Base, // InAxib
    CpuState::Base, // InEaxDx
    CpuState::Base, // InEaxib
    CpuState::Base, // OutDxAl
    CpuState::Base, // OutDxAx
    CpuState::Base, // OutDxEax
    CpuState::Base, // OutIbAl
    CpuState::Base, // OutIbAx
    CpuState::Base, // OutIbEax
    CpuState::Base, // IntIb
    CpuState::Base, // INT1
    CpuState::Base, // INT3
    CpuState::Base, // Int0
    CpuState::Base, // IretOp16
    CpuState::Base, // IretOp32
    CpuState::Base, // JmpEd
    CpuState::Base, // JmpEw
    CpuState::Base, // JmpJw
    CpuState::Base, // JmpJbw
    CpuState::Base, // JmpJd
    CpuState::Base, // JmpJbd
    CpuState::Base, // JmpfAp
    CpuState::Base, // JmpfOp16Ep
    CpuState::Base, // JmpfOp32Ep
    CpuState::Base, // JcxzJbw
    CpuState::Base, // JecxzJbd
    CpuState::Base, // LoopJbw
    CpuState::Base, // LoopeJbw
    CpuState::Base, // LoopneJbw
    CpuState::Base, // LoopJbd
    CpuState::Base, // LoopeJbd
    CpuState::Base, // LoopneJbd
    CpuState::Base, // JbJw
    CpuState::Base, // JbeJw
    CpuState::Base, // JlJw
    CpuState::Base, // JleJw
    CpuState::Base, // JnbJw
    CpuState::Base, // JnbeJw
    CpuState::Base, // JnlJw
    CpuState::Base, // JnleJw
    CpuState::Base, // JnoJw
    CpuState::Base, // JnpJw
    CpuState::Base, // JnsJw
    CpuState::Base, // JnzJw
    CpuState::Base, // JoJw
    CpuState::Base, // JpJw
    CpuState::Base, // JsJw
    CpuState::Base, // JzJw
    CpuState::Base, // JbJbw
    CpuState::Base, // JbeJbw
    CpuState::Base, // JlJbw
    CpuState::Base, // JleJbw
    CpuState::Base, // JnbJbw
    CpuState::Base, // JnbeJbw
    CpuState::Base, // JnlJbw
    CpuState::Base, // JnleJbw
    CpuState::Base, // JnoJbw
    CpuState::Base, // JnpJbw
    CpuState::Base, // JnsJbw
    CpuState::Base, // JnzJbw
    CpuState::Base, // JoJbw
    CpuState::Base, // JpJbw
    CpuState::Base, // JsJbw
    CpuState::Base, // JzJbw
    CpuState::Base, // JbJd
    CpuState::Base, // JbeJd
    CpuState::Base, // JlJd
    CpuState::Base, // JleJd
    CpuState::Base, // JnbJd
    CpuState::Base, // JnbeJd
    CpuState::Base, // JnlJd
    CpuState::Base, // JnleJd
    CpuState::Base, // JnoJd
    CpuState::Base, // JnpJd
    CpuState::Base, // JnsJd
    CpuState::Base, // JnzJd
    CpuState::Base, // JoJd
    CpuState::Base, // JpJd
    CpuState::Base, // JsJd
    CpuState::Base, // JzJd
    CpuState::Base, // JbJbd
    CpuState::Base, // JbeJbd
    CpuState::Base, // JlJbd
    CpuState::Base, // JleJbd
    CpuState::Base, // JnbJbd
    CpuState::Base, // JnbeJbd
    CpuState::Base, // JnlJbd
    CpuState::Base, // JnleJbd
    CpuState::Base, // JnoJbd
    CpuState::Base, // JnpJbd
    CpuState::Base, // JnsJbd
    CpuState::Base, // JnzJbd
    CpuState::Base, // JoJbd
    CpuState::Base, // JpJbd
    CpuState::Base, // JsJbd
    CpuState::Base, // JzJbd
    CpuState::Base, // Sahf
    CpuState::Base, // Lahf
    CpuState::Base, // LdsGdMp
    CpuState::Base, // LdsGwMp
    CpuState::Base, // LesGdMp
    CpuState::Base, // LesGwMp
    CpuState::Base, // LfsGdMp
    CpuState::Base, // LfsGwMp
    CpuState::Base, // LssGdMp
    CpuState::Base, // LssGwMp
    CpuState::Base, // LgsGdMp
    CpuState::Base, // LgsGwMp
    CpuState::Base, // LarGwEw
    CpuState::Base, // LslGwEw
    CpuState::Base, // LarGdEw
    CpuState::Base, // LslGdEw
    CpuState::Base, // LeaGdM
    CpuState::Base, // LeaGwM
    CpuState::Base, // SidtMs
    CpuState::Base, // LidtMs
    CpuState::Base, // SgdtMs
    CpuState::Base, // LgdtMs
    CpuState::Base, // SldtEw
    CpuState::Base, // LldtEw
    CpuState::Base, // StrEw
    CpuState::Base, // LtrEw
    CpuState::Base, // SmswEw
    CpuState::Base, // LmswEw
    CpuState::Base, // MovCr0rd
    CpuState::Base, // MovCr2rd
    CpuState::Base, // MovCr3rd
    CpuState::Base, // MovCr4rd
    CpuState::Base, // MovRdCr0
    CpuState::Base, // MovRdCr2
    CpuState::Base, // MovRdCr3
    CpuState::Base, // MovRdCr4
    CpuState::Base, // MovRdDd
    CpuState::Base, // MovDdRd
    CpuState::Base, // MovEbIb
    CpuState::Base, // MovEdId
    CpuState::Base, // MovEwIw
    CpuState::Base, // MovGbEb
    CpuState::Base, // MovEbGb
    CpuState::Base, // MovGwEw
    CpuState::Base, // MovEwGw
    CpuState::Base, // MovOp32GdEd
    CpuState::Base, // MovOp32EdGd
    CpuState::Base, // MovEwSw
    CpuState::Base, // MovSwEw
    CpuState::Base, // MovAlod
    CpuState::Base, // MovAxod
    CpuState::Base, // MovEaxod
    CpuState::Base, // MovOdAl
    CpuState::Base, // MovOdAx
    CpuState::Base, // MovOdEax
    CpuState::Base, // MovsxGdEb
    CpuState::Base, // MovsxGdEw
    CpuState::Base, // MovsxGwEb
    CpuState::Base, // MovzxGdEb
    CpuState::Base, // MovzxGdEw
    CpuState::Base, // MovzxGwEb
    CpuState::Base, // Nop
    CpuState::Base, // Pause
    CpuState::Base, // PopEw
    CpuState::Base, // PopEd
    CpuState::Base, // PopOp16Sw
    CpuState::Base, // PopOp32Sw
    CpuState::Base, // PopaOp16
    CpuState::Base, // PopaOp32
    CpuState::Base, // PopfFw
    CpuState::Base, // PopfFd
    CpuState::Base, // PushEw
    CpuState::Base, // PushEd
    CpuState::Base, // PushId
    CpuState::Base, // PushSIb32
    CpuState::Base, // PushIw
    CpuState::Base, // PushSIb16
    CpuState::Base, // PushOp16Sw
    CpuState::Base, // PushOp32Sw
    CpuState::Base, // PushaOp16
    CpuState::Base, // PushaOp32
    CpuState::Base, // PushfFw
    CpuState::Base, // PushfFd
    CpuState::Base, // RepCmpsbXbYb
    CpuState::Base, // RepCmpsdXdYd
    CpuState::Base, // RepCmpswXwYw
    CpuState::Base, // RepInsbYbDx
    CpuState::Base, // RepInsdYdDx
    CpuState::Base, // RepInswYwDx
    CpuState::Base, // RepLodsbAlxb
    CpuState::Base, // RepLodsdEaxxd
    CpuState::Base, // RepLodswAxxw
    CpuState::Base, // RepMovsbYbXb
    CpuState::Base, // RepMovsdYdXd
    CpuState::Base, // RepMovswYwXw
    CpuState::Base, // RepOutsbDxxb
    CpuState::Base, // RepOutsdDxxd
    CpuState::Base, // RepOutswDxxw
    CpuState::Base, // RepScasbAlyb
    CpuState::Base, // RepScasdEaxyd
    CpuState::Base, // RepScaswAxyw
    CpuState::Base, // RepStosbYbAl
    CpuState::Base, // RepStosdYdEax
    CpuState::Base, // RepStoswYwAx
    CpuState::Base, // RetfOp16
    CpuState::Base, // RetfOp16Iw
    CpuState::Base, // RetfOp32
    CpuState::Base, // RetfOp32Iw
    CpuState::Base, // RetOp16
    CpuState::Base, // RetOp16Iw
    CpuState::Base, // RetOp32
    CpuState::Base, // RetOp32Iw
    CpuState::Base, // NotEb
    CpuState::Base, // NegEb
    CpuState::Base, // NotEw
    CpuState::Base, // NegEw
    CpuState::Base, // NotEd
    CpuState::Base, // NegEd
    CpuState::Base, // RolEb
    CpuState::Base, // RorEb
    CpuState::Base, // RclEb
    CpuState::Base, // RcrEb
    CpuState::Base, // ShlEb
    CpuState::Base, // ShrEb
    CpuState::Base, // SarEb
    CpuState::Base, // RolEw
    CpuState::Base, // RorEw
    CpuState::Base, // RclEw
    CpuState::Base, // RcrEw
    CpuState::Base, // ShlEw
    CpuState::Base, // ShrEw
    CpuState::Base, // SarEw
    CpuState::Base, // RolEd
    CpuState::Base, // RorEd
    CpuState::Base, // RclEd
    CpuState::Base, // RcrEd
    CpuState::Base, // ShlEd
    CpuState::Base, // ShrEd
    CpuState::Base, // SarEd
    CpuState::Base, // RolEbIb
    CpuState::Base, // RorEbIb
    CpuState::Base, // RclEbIb
    CpuState::Base, // RcrEbIb
    CpuState::Base, // ShlEbIb
    CpuState::Base, // ShrEbIb
    CpuState::Base, // SarEbIb
    CpuState::Base, // RolEwIb
    CpuState::Base, // RorEwIb
    CpuState::Base, // RclEwIb
    CpuState::Base, // RcrEwIb
    CpuState::Base, // ShlEwIb
    CpuState::Base, // ShrEwIb
    CpuState::Base, // SarEwIb
    CpuState::Base, // RolEdIb
    CpuState::Base, // RorEdIb
    CpuState::Base, // RclEdIb
    CpuState::Base, // RcrEdIb
    CpuState::Base, // ShlEdIb
    CpuState::Base, // ShrEdIb
    CpuState::Base, // SarEdIb
    CpuState::Base, // RolEbI1
    CpuState::Base, // RorEbI1
    CpuState::Base, // RclEbI1
    CpuState::Base, // RcrEbI1
    CpuState::Base, // ShlEbI1
    CpuState::Base, // ShrEbI1
    CpuState::Base, // SarEbI1
    CpuState::Base, // RolEwI1
    CpuState::Base, // RorEwI1
    CpuState::Base, // RclEwI1
    CpuState::Base, // RcrEwI1
    CpuState::Base, // ShlEwI1
    CpuState::Base, // ShrEwI1
    CpuState::Base, // SarEwI1
    CpuState::Base, // RolEdI1
    CpuState::Base, // RorEdI1
    CpuState::Base, // RclEdI1
    CpuState::Base, // RcrEdI1
    CpuState::Base, // ShlEdI1
    CpuState::Base, // ShrEdI1
    CpuState::Base, // SarEdI1
    CpuState::Base, // SetbEb
    CpuState::Base, // SetbeEb
    CpuState::Base, // SetlEb
    CpuState::Base, // SetleEb
    CpuState::Base, // SetnbEb
    CpuState::Base, // SetnbeEb
    CpuState::Base, // SetnlEb
    CpuState::Base, // SetnleEb
    CpuState::Base, // SetnoEb
    CpuState::Base, // SetnpEb
    CpuState::Base, // SetnsEb
    CpuState::Base, // SetnzEb
    CpuState::Base, // SetoEb
    CpuState::Base, // SetpEb
    CpuState::Base, // SetsEb
    CpuState::Base, // SetzEb
    CpuState::Base, // ShldEdGd
    CpuState::Base, // ShldEdGdIb
    CpuState::Base, // ShldEwGw
    CpuState::Base, // ShldEwGwIb
    CpuState::Base, // ShrdEdGd
    CpuState::Base, // ShrdEdGdIb
    CpuState::Base, // ShrdEwGw
    CpuState::Base, // ShrdEwGwIb
    CpuState::Base, // Rsm
    CpuState::Base, // Salc
    CpuState::Base, // Stc
    CpuState::Base, // Std
    CpuState::Base, // Sti
    CpuState::Base, // MulAleb
    CpuState::Base, // ImulAleb
    CpuState::Base, // DivAleb
    CpuState::Base, // IdivAleb
    CpuState::Base, // MulAxew
    CpuState::Base, // ImulAxew
    CpuState::Base, // DivAxew
    CpuState::Base, // IdivAxew
    CpuState::Base, // MulEaxed
    CpuState::Base, // ImulEaxed
    CpuState::Base, // DivEaxed
    CpuState::Base, // IdivEaxed
    CpuState::Base, // VerrEw
    CpuState::Base, // VerwEw
    CpuState::Base, // XchgEbGb
    CpuState::Base, // XchgEwGw
    CpuState::Base, // XchgEdGd
    CpuState::Base, // XchgRxax
    CpuState::Base, // XchgErxEax
    CpuState::Base, // Xlat
    CpuState::Base, // Sysenter
    CpuState::Base, // Sysexit
    CpuState::Base, // Monitor
    CpuState::Base, // Mwait
    CpuState::Base, // UmonitorEq
    CpuState::Base, // UmonitorEd
    CpuState::Base, // UmwaitEd
    CpuState::Base, // TpauseEd
    CpuState::Base, // Monitorx
    CpuState::Base, // Mwaitx
    CpuState::Base, // Fwait
    CpuState::Fpu, // FldSti
    CpuState::Fpu, // FldSingleReal
    CpuState::Fpu, // FldDoubleReal
    CpuState::Fpu, // FldExtendedReal
    CpuState::Fpu, // FildWordInteger
    CpuState::Fpu, // FildDwordInteger
    CpuState::Fpu, // FildQwordInteger
    CpuState::Fpu, // FbldPackedBcd
    CpuState::Fpu, // FstSti
    CpuState::Fpu, // FstpSti
    CpuState::Fpu, // FstpSpecialSti
    CpuState::Fpu, // FstSingleReal
    CpuState::Fpu, // FstpSingleReal
    CpuState::Fpu, // FstDoubleReal
    CpuState::Fpu, // FstpDoubleReal
    CpuState::Fpu, // FstpExtendedReal
    CpuState::Fpu, // FistWordInteger
    CpuState::Fpu, // FistpWordInteger
    CpuState::Fpu, // FistDwordInteger
    CpuState::Fpu, // FistpDwordInteger
    CpuState::Fpu, // FistpQwordInteger
    CpuState::Fpu, // FbstpPackedBcd
    CpuState::Fpu, // FisttpMw
    CpuState::Fpu, // FisttpMd
    CpuState::Fpu, // FisttpMq
    CpuState::Fpu, // Fninit
    CpuState::Fpu, // Fnclex
    CpuState::Fpu, // Frstor
    CpuState::Fpu, // Fnsave
    CpuState::Fpu, // Fldenv
    CpuState::Fpu, // Fnstenv
    CpuState::Fpu, // Fldcw
    CpuState::Fpu, // Fnstcw
    CpuState::Fpu, // Fnstsw
    CpuState::Fpu, // FnstswAx
    CpuState::Fpu, // FLD1
    CpuState::Fpu, // Fldl2t
    CpuState::Fpu, // Fldl2e
    CpuState::Fpu, // Fldpi
    CpuState::Fpu, // Fldlg2
    CpuState::Fpu, // Fldln2
    CpuState::Fpu, // Fldz
    CpuState::Fpu, // FaddSt0Stj
    CpuState::Fpu, // FaddStiSt0
    CpuState::Fpu, // FaddpStiSt0
    CpuState::Fpu, // FaddSingleReal
    CpuState::Fpu, // FaddDoubleReal
    CpuState::Fpu, // FiaddWordInteger
    CpuState::Fpu, // FiaddDwordInteger
    CpuState::Fpu, // FmulSt0Stj
    CpuState::Fpu, // FmulStiSt0
    CpuState::Fpu, // FmulpStiSt0
    CpuState::Fpu, // FmulSingleReal
    CpuState::Fpu, // FmulDoubleReal
    CpuState::Fpu, // FimulWordInteger
    CpuState::Fpu, // FimulDwordInteger
    CpuState::Fpu, // FsubSt0Stj
    CpuState::Fpu, // FsubrSt0Stj
    CpuState::Fpu, // FsubStiSt0
    CpuState::Fpu, // FsubpStiSt0
    CpuState::Fpu, // FsubrStiSt0
    CpuState::Fpu, // FsubrpStiSt0
    CpuState::Fpu, // FsubSingleReal
    CpuState::Fpu, // FsubrSingleReal
    CpuState::Fpu, // FsubDoubleReal
    CpuState::Fpu, // FsubrDoubleReal
    CpuState::Fpu, // FisubWordInteger
    CpuState::Fpu, // FisubrWordInteger
    CpuState::Fpu, // FisubDwordInteger
    CpuState::Fpu, // FisubrDwordInteger
    CpuState::Fpu, // FdivSt0Stj
    CpuState::Fpu, // FdivrSt0Stj
    CpuState::Fpu, // FdivStiSt0
    CpuState::Fpu, // FdivpStiSt0
    CpuState::Fpu, // FdivrStiSt0
    CpuState::Fpu, // FdivrpStiSt0
    CpuState::Fpu, // FdivSingleReal
    CpuState::Fpu, // FdivrSingleReal
    CpuState::Fpu, // FdivDoubleReal
    CpuState::Fpu, // FdivrDoubleReal
    CpuState::Fpu, // FidivWordInteger
    CpuState::Fpu, // FidivrWordInteger
    CpuState::Fpu, // FidivDwordInteger
    CpuState::Fpu, // FidivrDwordInteger
    CpuState::Fpu, // FcomSti
    CpuState::Fpu, // FcompSti
    CpuState::Fpu, // FucomSti
    CpuState::Fpu, // FucompSti
    CpuState::Fpu, // FcomiSt0Stj
    CpuState::Fpu, // FcomipSt0Stj
    CpuState::Fpu, // FucomiSt0Stj
    CpuState::Fpu, // FucomipSt0Stj
    CpuState::Fpu, // FcomSingleReal
    CpuState::Fpu, // FcompSingleReal
    CpuState::Fpu, // FcomDoubleReal
    CpuState::Fpu, // FcompDoubleReal
    CpuState::Fpu, // FicomWordInteger
    CpuState::Fpu, // FicompWordInteger
    CpuState::Fpu, // FicomDwordInteger
    CpuState::Fpu, // FicompDwordInteger
    CpuState::Fpu, // FcmovbSt0Stj
    CpuState::Fpu, // FcmoveSt0Stj
    CpuState::Fpu, // FcmovbeSt0Stj
    CpuState::Fpu, // FcmovuSt0Stj
    CpuState::Fpu, // FcmovnbSt0Stj
    CpuState::Fpu, // FcmovneSt0Stj
    CpuState::Fpu, // FcmovnbeSt0Stj
    CpuState::Fpu, // FcmovnuSt0Stj
    CpuState::Fpu, // Fcompp
    CpuState::Fpu, // Fucompp
    CpuState::Fpu, // FxchSti
    CpuState::Fpu, // Fnop
    CpuState::Fpu, // Fplegacy
    CpuState::Fpu, // Fchs
    CpuState::Fpu, // Fabs
    CpuState::Fpu, // Ftst
    CpuState::Fpu, // Fxam
    CpuState::Fpu, // Fdecstp
    CpuState::Fpu, // Fincstp
    CpuState::Fpu, // FfreeSti
    CpuState::Fpu, // FfreepSti
    CpuState::Fpu, // F2XM1
    CpuState::Fpu, // FYL2X
    CpuState::Fpu, // Fptan
    CpuState::Fpu, // Fpatan
    CpuState::Fpu, // Fxtract
    CpuState::Fpu, // FPREM1
    CpuState::Fpu, // Fprem
    CpuState::Fpu, // FYL2XP1
    CpuState::Fpu, // Fsqrt
    CpuState::Fpu, // Fsincos
    CpuState::Fpu, // Frndint
    CpuState::Fpu, // Fscale
    CpuState::Fpu, // Fsin
    CpuState::Fpu, // Fcos
    CpuState::Fpu, // Fpuesc
    CpuState::Base, // Cpuid
    CpuState::Base, // BswapRx
    CpuState::Base, // BswapErx
    CpuState::Base, // Invd
    CpuState::Base, // Wbinvd
    CpuState::Base, // XaddEbGb
    CpuState::Base, // XaddEwGw
    CpuState::Base, // XaddEdGd
    CpuState::Base, // CmpxchgEbGb
    CpuState::Base, // CmpxchgEwGw
    CpuState::Base, // CmpxchgEdGd
    CpuState::Base, // Invlpg
    CpuState::Base, // Cmpxchg8b
    CpuState::Base, // Wrmsr
    CpuState::Base, // Rdmsr
    CpuState::Base, // Rdtsc
    CpuState::Mmx, // PunpcklbwPqQd
    CpuState::Mmx, // PunpcklwdPqQd
    CpuState::Mmx, // PunpckldqPqQd
    CpuState::Mmx, // PacksswbPqQq
    CpuState::Mmx, // PcmpgtbPqQq
    CpuState::Mmx, // PcmpgtwPqQq
    CpuState::Mmx, // PcmpgtdPqQq
    CpuState::Mmx, // PackuswbPqQq
    CpuState::Mmx, // PunpckhbwPqQq
    CpuState::Mmx, // PunpckhwdPqQq
    CpuState::Mmx, // PunpckhdqPqQq
    CpuState::Mmx, // PackssdwPqQq
    CpuState::Mmx, // MovdPqEd
    CpuState::Mmx, // MovqPqQq
    CpuState::Mmx, // PcmpeqbPqQq
    CpuState::Mmx, // PcmpeqwPqQq
    CpuState::Mmx, // PcmpeqdPqQq
    CpuState::Mmx, // Emms
    CpuState::Mmx, // MovdEdPq
    CpuState::Mmx, // MovqQqPq
    CpuState::Mmx, // PsrlwPqQq
    CpuState::Mmx, // PsrldPqQq
    CpuState::Mmx, // PsrlqPqQq
    CpuState::Mmx, // PmullwPqQq
    CpuState::Mmx, // PsubusbPqQq
    CpuState::Mmx, // PsubuswPqQq
    CpuState::Mmx, // PandPqQq
    CpuState::Mmx, // PaddusbPqQq
    CpuState::Mmx, // PadduswPqQq
    CpuState::Mmx, // PandnPqQq
    CpuState::Mmx, // PsrawPqQq
    CpuState::Mmx, // PsradPqQq
    CpuState::Mmx, // PmulhwPqQq
    CpuState::Mmx, // PsubsbPqQq
    CpuState::Mmx, // PsubswPqQq
    CpuState::Mmx, // PorPqQq
    CpuState::Mmx, // PaddsbPqQq
    CpuState::Mmx, // PaddswPqQq
    CpuState::Mmx, // PxorPqQq
    CpuState::Mmx, // PsllwPqQq
    CpuState::Mmx, // PslldPqQq
    CpuState::Mmx, // PsllqPqQq
    CpuState::Mmx, // PmaddwdPqQq
    CpuState::Mmx, // PsubbPqQq
    CpuState::Mmx, // PsubwPqQq
    CpuState::Mmx, // PsubdPqQq
    CpuState::Mmx, // PaddbPqQq
    CpuState::Mmx, // PaddwPqQq
    CpuState::Mmx, // PadddPqQq
    CpuState::Mmx, // PsrlwNqIb
    CpuState::Mmx, // PsrawNqIb
    CpuState::Mmx, // PsllwNqIb
    CpuState::Mmx, // PsrldNqIb
    CpuState::Mmx, // PsradNqIb
    CpuState::Mmx, // PslldNqIb
    CpuState::Mmx, // PsrlqNqIb
    CpuState::Mmx, // PsllqNqIb
    CpuState::Mmx, // MovqEqPq
    CpuState::Mmx, // Femms
    CpuState::Mmx, // Pf2idPqQq
    CpuState::Mmx, // Pf2iwPqQq
    CpuState::Mmx, // PfaccPqQq
    CpuState::Mmx, // PfaddPqQq
    CpuState::Mmx, // PfcmpeqPqQq
    CpuState::Mmx, // PfcmpgePqQq
    CpuState::Mmx, // PfcmpgtPqQq
    CpuState::Mmx, // PfmaxPqQq
    CpuState::Mmx, // PfminPqQq
    CpuState::Mmx, // PfmulPqQq
    CpuState::Mmx, // PfnaccPqQq
    CpuState::Mmx, // PfpnaccPqQq
    CpuState::Mmx, // PfrcpPqQq
    CpuState::Mmx, // Pfrcpit1PqQq
    CpuState::Mmx, // Pfrcpit2PqQq
    CpuState::Mmx, // Pfrsqit1PqQq
    CpuState::Mmx, // PfrsqrtPqQq
    CpuState::Mmx, // PfsubPqQq
    CpuState::Mmx, // PfsubrPqQq
    CpuState::Mmx, // Pi2fdPqQq
    CpuState::Mmx, // Pi2fwPqQq
    CpuState::Mmx, // PmulhrwPqQq
    CpuState::Mmx, // PswapdPqQq
    CpuState::Base, // PrefetchwMb
    CpuState::Base, // SyscallLegacy
    CpuState::Base, // SysretLegacy
    CpuState::Base, // CmovbGwEw
    CpuState::Base, // CmovbeGwEw
    CpuState::Base, // CmovlGwEw
    CpuState::Base, // CmovleGwEw
    CpuState::Base, // CmovnbGwEw
    CpuState::Base, // CmovnbeGwEw
    CpuState::Base, // CmovnlGwEw
    CpuState::Base, // CmovnleGwEw
    CpuState::Base, // CmovnoGwEw
    CpuState::Base, // CmovnpGwEw
    CpuState::Base, // CmovnsGwEw
    CpuState::Base, // CmovnzGwEw
    CpuState::Base, // CmovoGwEw
    CpuState::Base, // CmovpGwEw
    CpuState::Base, // CmovsGwEw
    CpuState::Base, // CmovzGwEw
    CpuState::Base, // CmovbGdEd
    CpuState::Base, // CmovbeGdEd
    CpuState::Base, // CmovlGdEd
    CpuState::Base, // CmovleGdEd
    CpuState::Base, // CmovnbGdEd
    CpuState::Base, // CmovnbeGdEd
    CpuState::Base, // CmovnlGdEd
    CpuState::Base, // CmovnleGdEd
    CpuState::Base, // CmovnoGdEd
    CpuState::Base, // CmovnpGdEd
    CpuState::Base, // CmovnsGdEd
    CpuState::Base, // CmovnzGdEd
    CpuState::Base, // CmovoGdEd
    CpuState::Base, // CmovpGdEd
    CpuState::Base, // CmovsGdEd
    CpuState::Base, // CmovzGdEd
    CpuState::Base, // Rdpmc
    CpuState::Base, // Ud0
    CpuState::Base, // Ud1
    CpuState::Base, // Ud2
    CpuState::Base, // Fxsave
    CpuState::Base, // Fxrstor
    CpuState::Sse, // Ldmxcsr
    CpuState::Sse, // Stmxcsr
    CpuState::Base, // PrefetchMb
    CpuState::Base, // Prefetcht0Mb
    CpuState::Base, // Prefetcht1Mb
    CpuState::Base, // Prefetcht2Mb
    CpuState::Base, // PrefetchntaMb
    CpuState::Sse, // AndpsVpsWps
    CpuState::Sse, // OrpsVpsWps
    CpuState::Sse, // XorpsVpsWps
    CpuState::Sse, // AndnpsVpsWps
    CpuState::Sse, // MovupsVpsWps
    CpuState::Sse, // MovupsWpsVps
    CpuState::Sse, // MovssVssWss
    CpuState::Sse, // MovssWssVss
    CpuState::Sse, // MovlpsVpsMq
    CpuState::Sse, // MovhlpsVpsWps
    CpuState::Sse, // MovlpsMqVps
    CpuState::Sse, // MovhpsVpsMq
    CpuState::Sse, // MovlhpsVpsWps
    CpuState::Sse, // MovhpsMqVps
    CpuState::Sse, // MovapsVpsWps
    CpuState::Sse, // MovapsWpsVps
    CpuState::Sse, // MovntpsMpsVps
    CpuState::Sse, // Cvtpi2psVpsQq
    CpuState::Sse, // Cvtsi2ssVssEd
    CpuState::Sse, // Cvttps2piPqWps
    CpuState::Sse, // Cvtps2piPqWps
    CpuState::Sse, // Cvttss2siGdWss
    CpuState::Sse, // Cvtss2siGdWss
    CpuState::Sse, // UcomissVssWss
    CpuState::Sse, // ComissVssWss
    CpuState::Sse, // MovmskpsGdUps
    CpuState::Sse, // MovmskpdGdUpd
    CpuState::Sse, // RsqrtpsVpsWps
    CpuState::Sse, // RsqrtssVssWss
    CpuState::Sse, // RcppsVpsWps
    CpuState::Sse, // RcpssVssWss
    CpuState::Mmx, // PshufwPqQqIb
    CpuState::Sse, // PshuflwVdqWdqIb
    CpuState::Mmx, // PinsrwPqEwIb
    CpuState::Mmx, // PextrwGdNqIb
    CpuState::Sse, // ShufpsVpsWpsIb
    CpuState::Mmx, // PmovmskbGdNq
    CpuState::Mmx, // PminubPqQq
    CpuState::Mmx, // PmaxubPqQq
    CpuState::Mmx, // PavgbPqQq
    CpuState::Mmx, // PavgwPqQq
    CpuState::Mmx, // PmulhuwPqQq
    CpuState::Mmx, // MovntqMqPq
    CpuState::Mmx, // PminswPqQq
    CpuState::Mmx, // PmaxswPqQq
    CpuState::Mmx, // PsadbwPqQq
    CpuState::Mmx, // MaskmovqPqNq
    CpuState::Sse, // AddpsVpsWps
    CpuState::Sse, // AddpdVpdWpd
    CpuState::Sse, // AddssVssWss
    CpuState::Sse, // AddsdVsdWsd
    CpuState::Sse, // MulpsVpsWps
    CpuState::Sse, // MulpdVpdWpd
    CpuState::Sse, // MulssVssWss
    CpuState::Sse, // MulsdVsdWsd
    CpuState::Sse, // SubpsVpsWps
    CpuState::Sse, // SubpdVpdWpd
    CpuState::Sse, // SubssVssWss
    CpuState::Sse, // SubsdVsdWsd
    CpuState::Sse, // MinpsVpsWps
    CpuState::Sse, // MinpdVpdWpd
    CpuState::Sse, // MinssVssWss
    CpuState::Sse, // MinsdVsdWsd
    CpuState::Sse, // DivpsVpsWps
    CpuState::Sse, // DivpdVpdWpd
    CpuState::Sse, // DivssVssWss
    CpuState::Sse, // DivsdVsdWsd
    CpuState::Sse, // MaxpsVpsWps
    CpuState::Sse, // MaxpdVpdWpd
    CpuState::Sse, // MaxssVssWss
    CpuState::Sse, // MaxsdVsdWsd
    CpuState::Sse, // SqrtpsVpsWps
    CpuState::Sse, // SqrtpdVpdWpd
    CpuState::Sse, // SqrtssVssWss
    CpuState::Sse, // SqrtsdVsdWsd
    CpuState::Sse, // CmppsVpsWpsIb
    CpuState::Sse, // CmppdVpdWpdIb
    CpuState::Sse, // CmpssVssWssIb
    CpuState::Sse, // CmpsdVsdWsdIb
    CpuState::Sse, // Cvtps2pdVpdWps
    CpuState::Sse, // Cvtpd2psVpsWpd
    CpuState::Sse, // Cvtss2sdVsdWss
    CpuState::Sse, // Cvtsd2ssVssWsd
    CpuState::Sse, // MovsdVsdWsd
    CpuState::Sse, // MovsdWsdVsd
    CpuState::Sse, // Cvtpi2pdVpdQq
    CpuState::Sse, // Cvtsi2sdVsdEd
    CpuState::Sse, // Cvttpd2piPqWpd
    CpuState::Sse, // Cvttsd2siGdWsd
    CpuState::Sse, // Cvtpd2piPqWpd
    CpuState::Sse, // Cvtsd2siGdWsd
    CpuState::Sse, // UcomisdVsdWsd
    CpuState::Sse, // ComisdVsdWsd
    CpuState::Sse, // Cvtdq2psVpsWdq
    CpuState::Sse, // Cvtps2dqVdqWps
    CpuState::Sse, // Cvttps2dqVdqWps
    CpuState::Sse, // UnpckhpdVpdWdq
    CpuState::Sse, // UnpcklpdVpdWdq
    CpuState::Sse, // PunpckhdqVdqWdq
    CpuState::Sse, // PunpckldqVdqWdq
    CpuState::Sse, // MovapdVpdWpd
    CpuState::Sse, // MovapdWpdVpd
    CpuState::Sse, // MovdqaVdqWdq
    CpuState::Sse, // MovdqaWdqVdq
    CpuState::Sse, // MovdquVdqWdq
    CpuState::Sse, // MovdquWdqVdq
    CpuState::Sse, // MovhpdMqVsd
    CpuState::Sse, // MovhpdVsdMq
    CpuState::Sse, // MovlpdMqVsd
    CpuState::Sse, // MovlpdVsdMq
    CpuState::Sse, // MovntdqMdqVdq
    CpuState::Sse, // MovntpdMpdVpd
    CpuState::Sse, // MovupdVpdWpd
    CpuState::Sse, // MovupdWpdVpd
    CpuState::Sse, // AndnpdVpdWpd
    CpuState::Sse, // AndpdVpdWpd
    CpuState::Sse, // OrpdVpdWpd
    CpuState::Sse, // XorpdVpdWpd
    CpuState::Sse, // PandVdqWdq
    CpuState::Sse, // PandnVdqWdq
    CpuState::Sse, // PorVdqWdq
    CpuState::Sse, // PxorVdqWdq
    CpuState::Sse, // PunpcklbwVdqWdq
    CpuState::Sse, // PunpcklwdVdqWdq
    CpuState::Sse, // UnpcklpsVpsWdq
    CpuState::Sse, // UnpckhpsVpsWdq
    CpuState::Sse, // PackuswbVdqWdq
    CpuState::Sse, // PacksswbVdqWdq
    CpuState::Sse, // PcmpgtbVdqWdq
    CpuState::Sse, // PcmpgtwVdqWdq
    CpuState::Sse, // PcmpgtdVdqWdq
    CpuState::Sse, // PunpckhbwVdqWdq
    CpuState::Sse, // PunpckhwdVdqWdq
    CpuState::Sse, // PackssdwVdqWdq
    CpuState::Sse, // PunpcklqdqVdqWdq
    CpuState::Sse, // PunpckhqdqVdqWdq
    CpuState::Sse, // MovdVdqEd
    CpuState::Sse, // PshufdVdqWdqIb
    CpuState::Sse, // PshufhwVdqWdqIb
    CpuState::Sse, // PcmpeqbVdqWdq
    CpuState::Sse, // PcmpeqwVdqWdq
    CpuState::Sse, // PcmpeqdVdqWdq
    CpuState::Sse, // MovdEdVd
    CpuState::Sse, // MovqVqWq
    CpuState::Base, // MovntiOp32MdGd
    CpuState::Sse, // PinsrwVdqEwIb
    CpuState::Sse, // PextrwGdUdqIb
    CpuState::Sse, // ShufpdVpdWpdIb
    CpuState::Sse, // PsrlwVdqWdq
    CpuState::Sse, // PsrldVdqWdq
    CpuState::Sse, // PsrlqVdqWdq
    CpuState::Mmx, // PaddqPqQq
    CpuState::Mmx, // PsubqPqQq
    CpuState::Sse, // PaddqVdqWdq
    CpuState::Sse, // PmullwVdqWdq
    CpuState::Sse, // MovqWqVq
    CpuState::Sse, // Movdq2qPqUdq
    CpuState::Sse, // Movq2dqVdqQq
    CpuState::Sse, // PmovmskbGdUdq
    CpuState::Sse, // PsubusbVdqWdq
    CpuState::Sse, // PsubuswVdqWdq
    CpuState::Sse, // PminubVdqWdq
    CpuState::Sse, // PaddusbVdqWdq
    CpuState::Sse, // PadduswVdqWdq
    CpuState::Sse, // PmaxubVdqWdq
    CpuState::Sse, // PavgbVdqWdq
    CpuState::Sse, // PsrawVdqWdq
    CpuState::Sse, // PsradVdqWdq
    CpuState::Sse, // PavgwVdqWdq
    CpuState::Sse, // PmulhuwVdqWdq
    CpuState::Sse, // PmulhwVdqWdq
    CpuState::Sse, // Cvttpd2dqVqWpd
    CpuState::Sse, // Cvtpd2dqVqWpd
    CpuState::Sse, // Cvtdq2pdVpdWq
    CpuState::Sse, // PsubsbVdqWdq
    CpuState::Sse, // PsubswVdqWdq
    CpuState::Sse, // PminswVdqWdq
    CpuState::Sse, // PmaxswVdqWdq
    CpuState::Sse, // PaddsbVdqWdq
    CpuState::Sse, // PaddswVdqWdq
    CpuState::Sse, // PsllwVdqWdq
    CpuState::Sse, // PslldVdqWdq
    CpuState::Sse, // PsllqVdqWdq
    CpuState::Mmx, // PmuludqPqQq
    CpuState::Sse, // PmuludqVdqWdq
    CpuState::Sse, // PmaddwdVdqWdq
    CpuState::Sse, // PsadbwVdqWdq
    CpuState::Sse, // MaskmovdquVdqUdq
    CpuState::Sse, // PsubbVdqWdq
    CpuState::Sse, // PsubwVdqWdq
    CpuState::Sse, // PsubdVdqWdq
    CpuState::Sse, // PsubqVdqWdq
    CpuState::Sse, // PaddbVdqWdq
    CpuState::Sse, // PaddwVdqWdq
    CpuState::Sse, // PadddVdqWdq
    CpuState::Sse, // PsrlwUdqIb
    CpuState::Sse, // PsrawUdqIb
    CpuState::Sse, // PsllwUdqIb
    CpuState::Sse, // PsrldUdqIb
    CpuState::Sse, // PsradUdqIb
    CpuState::Sse, // PslldUdqIb
    CpuState::Sse, // PsrlqUdqIb
    CpuState::Sse, // PsllqUdqIb
    CpuState::Sse, // PsrldqUdqIb
    CpuState::Sse, // PslldqUdqIb
    CpuState::Base, // Lfence
    CpuState::Base, // Sfence
    CpuState::Base, // Mfence
    CpuState::Sse, // MovddupVpdWq
    CpuState::Sse, // MovsldupVpsWps
    CpuState::Sse, // MovshdupVpsWps
    CpuState::Sse, // HaddpdVpdWpd
    CpuState::Sse, // HaddpsVpsWps
    CpuState::Sse, // HsubpdVpdWpd
    CpuState::Sse, // HsubpsVpsWps
    CpuState::Sse, // AddsubpdVpdWpd
    CpuState::Sse, // AddsubpsVpsWps
    CpuState::Sse, // LddquVdqMdq
    CpuState::Mmx, // PshufbPqQq
    CpuState::Mmx, // PhaddwPqQq
    CpuState::Mmx, // PhadddPqQq
    CpuState::Mmx, // PhaddswPqQq
    CpuState::Mmx, // PmaddubswPqQq
    CpuState::Mmx, // PhsubswPqQq
    CpuState::Mmx, // PhsubwPqQq
    CpuState::Mmx, // PhsubdPqQq
    CpuState::Mmx, // PsignbPqQq
    CpuState::Mmx, // PsignwPqQq
    CpuState::Mmx, // PsigndPqQq
    CpuState::Mmx, // PmulhrswPqQq
    CpuState::Mmx, // PabsbPqQq
    CpuState::Mmx, // PabswPqQq
    CpuState::Mmx, // PabsdPqQq
    CpuState::Mmx, // PalignrPqQqIb
    CpuState::Sse, // PshufbVdqWdq
    CpuState::Sse, // PhaddwVdqWdq
    CpuState::Sse, // PhadddVdqWdq
    CpuState::Sse, // PhaddswVdqWdq
    CpuState::Sse, // PmaddubswVdqWdq
    CpuState::Sse, // PhsubswVdqWdq
    CpuState::Sse, // PhsubwVdqWdq
    CpuState::Sse, // PhsubdVdqWdq
    CpuState::Sse, // PsignbVdqWdq
    CpuState::Sse, // PsignwVdqWdq
    CpuState::Sse, // PsigndVdqWdq
    CpuState::Sse, // PmulhrswVdqWdq
    CpuState::Sse, // PabsbVdqWdq
    CpuState::Sse, // PabswVdqWdq
    CpuState::Sse, // PabsdVdqWdq
    CpuState::Sse, // PalignrVdqWdqIb
    CpuState::Sse, // PblendvbVdqWdq
    CpuState::Sse, // BlendvpsVpsWps
    CpuState::Sse, // BlendvpdVpdWpd
    CpuState::Sse, // PmovsxbwVdqWq
    CpuState::Sse, // PmovsxbdVdqWd
    CpuState::Sse, // PmovsxbqVdqWw
    CpuState::Sse, // PmovsxwdVdqWq
    CpuState::Sse, // PmovsxwqVdqWd
    CpuState::Sse, // PmovsxdqVdqWq
    CpuState::Sse, // PmovzxbwVdqWq
    CpuState::Sse, // PmovzxbdVdqWd
    CpuState::Sse, // PmovzxbqVdqWw
    CpuState::Sse, // PmovzxwdVdqWq
    CpuState::Sse, // PmovzxwqVdqWd
    CpuState::Sse, // PmovzxdqVdqWq
    CpuState::Sse, // PtestVdqWdq
    CpuState::Sse, // PmuldqVdqWdq
    CpuState::Sse, // PcmpeqqVdqWdq
    CpuState::Sse, // PackusdwVdqWdq
    CpuState::Sse, // PminsbVdqWdq
    CpuState::Sse, // PminsdVdqWdq
    CpuState::Sse, // PminuwVdqWdq
    CpuState::Sse, // PminudVdqWdq
    CpuState::Sse, // PmaxsbVdqWdq
    CpuState::Sse, // PmaxsdVdqWdq
    CpuState::Sse, // PmaxuwVdqWdq
    CpuState::Sse, // PmaxudVdqWdq
    CpuState::Sse, // PmulldVdqWdq
    CpuState::Sse, // PhminposuwVdqWdq
    CpuState::Sse, // RoundpsVpsWpsIb
    CpuState::Sse, // RoundpdVpdWpdIb
    CpuState::Sse, // RoundssVssWssIb
    CpuState::Sse, // RoundsdVsdWsdIb
    CpuState::Sse, // BlendpsVpsWpsIb
    CpuState::Sse, // BlendpdVpdWpdIb
    CpuState::Sse, // PblendwVdqWdqIb
    CpuState::Sse, // PextrbEdVdqIbR
    CpuState::Sse, // PextrbMbVdqIbM
    CpuState::Sse, // PextrwEdVdqIbR
    CpuState::Sse, // PextrwMwVdqIbM
    CpuState::Sse, // PextrdEdVdqIb
    CpuState::Sse, // PextrqEqVdqIb
    CpuState::Sse, // ExtractpsEdVpsIb
    CpuState::Sse, // PinsrbVdqEbIb
    CpuState::Sse, // InsertpsVpsWssIb
    CpuState::Sse, // PinsrdVdqEdIb
    CpuState::Sse, // PinsrqVdqEqIb
    CpuState::Sse, // DppsVpsWpsIb
    CpuState::Sse, // DppdVpdWpdIb
    CpuState::Sse, // MpsadbwVdqWdqIb
    CpuState::Sse, // MovntdqaVdqMdq
    CpuState::Base, // Crc32GdEb
    CpuState::Base, // Crc32GdEw
    CpuState::Base, // Crc32GdEd
    CpuState::Base, // Crc32GdEq
    CpuState::Sse, // PcmpgtqVdqWdq
    CpuState::Sse, // PcmpestrmVdqWdqIb
    CpuState::Sse, // PcmpestriVdqWdqIb
    CpuState::Sse, // PcmpistrmVdqWdqIb
    CpuState::Sse, // PcmpistriVdqWdqIb
    CpuState::Base, // MovbeGwMw
    CpuState::Base, // MovbeGdMd
    CpuState::Base, // MovbeGqMq
    CpuState::Base, // MovbeMwGw
    CpuState::Base, // MovbeMdGd
    CpuState::Base, // MovbeMqGq
    CpuState::Base, // PopcntGwEw
    CpuState::Base, // PopcntGdEd
    CpuState::Base, // PopcntGqEq
    CpuState::Base, // Xrstor
    CpuState::Base, // Xsave
    CpuState::Base, // Xsavec
    CpuState::Base, // Xsetbv
    CpuState::Base, // Xgetbv
    CpuState::Base, // Xsaveopt
    CpuState::Base, // Xsaves
    CpuState::Base, // Xrstors
    CpuState::Sse, // AesimcVdqWdq
    CpuState::Sse, // AeskeygenassistVdqWdqIb
    CpuState::Sse, // AesencVdqWdq
    CpuState::Sse, // AesenclastVdqWdq
    CpuState::Sse, // AesdecVdqWdq
    CpuState::Sse, // AesdeclastVdqWdq
    CpuState::Sse, // PclmulqdqVdqWdqIb
    CpuState::Sse, // Sha1nexteVdqWdq
    CpuState::Sse, // Sha1msg1VdqWdq
    CpuState::Sse, // Sha1msg2VdqWdq
    CpuState::Sse, // Sha256rnds2VdqWdq
    CpuState::Sse, // Sha256msg1VdqWdq
    CpuState::Sse, // Sha256msg2VdqWdq
    CpuState::Sse, // Sha1rnds4VdqWdqIb
    CpuState::Sse, // Gf2p8affineqbVdqWdqIb
    CpuState::Sse, // Gf2p8affineinvqbVdqWdqIb
    CpuState::Sse, // Gf2p8mulbVdqWdq
    CpuState::Base, // LahfLm
    CpuState::Base, // SahfLm
    CpuState::Base, // Syscall
    CpuState::Base, // Sysret
    CpuState::Base, // XorEqGqZeroIdiom
    CpuState::Base, // XorGqEqZeroIdiom
    CpuState::Base, // SubEqGqZeroIdiom
    CpuState::Base, // SubGqEqZeroIdiom
    CpuState::Base, // AddGqEq
    CpuState::Base, // OrGqEq
    CpuState::Base, // AdcGqEq
    CpuState::Base, // SbbGqEq
    CpuState::Base, // AndGqEq
    CpuState::Base, // SubGqEq
    CpuState::Base, // XorGqEq
    CpuState::Base, // CmpGqEq
    CpuState::Base, // AddEqGq
    CpuState::Base, // OrEqGq
    CpuState::Base, // AdcEqGq
    CpuState::Base, // SbbEqGq
    CpuState::Base, // AndEqGq
    CpuState::Base, // SubEqGq
    CpuState::Base, // XorEqGq
    CpuState::Base, // TestEqGq
    CpuState::Base, // CmpEqGq
    CpuState::Base, // AddRaxid
    CpuState::Base, // OrRaxid
    CpuState::Base, // AdcRaxid
    CpuState::Base, // SbbRaxid
    CpuState::Base, // AndRaxid
    CpuState::Base, // SubRaxid
    CpuState::Base, // XorRaxid
    CpuState::Base, // TestRaxid
    CpuState::Base, // CmpRaxid
    CpuState::Base, // AddEqId
    CpuState::Base, // OrEqId
    CpuState::Base, // AdcEqId
    CpuState::Base, // SbbEqId
    CpuState::Base, // AndEqId
    CpuState::Base, // SubEqId
    CpuState::Base, // XorEqId
    CpuState::Base, // TestEqId
    CpuState::Base, // CmpEqId
    CpuState::Base, // AddEqsIb
    CpuState::Base, // OrEqsIb
    CpuState::Base, // AdcEqsIb
    CpuState::Base, // SbbEqsIb
    CpuState::Base, // AndEqsIb
    CpuState::Base, // SubEqsIb
    CpuState::Base, // XorEqsIb
    CpuState::Base, // TestEqsIb
    CpuState::Base, // CmpEqsIb
    CpuState::Base, // XchgEqGq
    CpuState::Base, // XchgRrxRax
    CpuState::Base, // LeaGqM
    CpuState::Base, // MovOp64GdEd
    CpuState::Base, // MovOp64EdGd
    CpuState::Base, // MovGqEq
    CpuState::Base, // MovEqGq
    CpuState::Base, // MovEqId
    CpuState::Base, // MovRaxoq
    CpuState::Base, // MovOqRax
    CpuState::Base, // MovEaxoq
    CpuState::Base, // MovOqEax
    CpuState::Base, // MovAxoq
    CpuState::Base, // MovOqAx
    CpuState::Base, // MovAloq
    CpuState::Base, // MovOqAl
    CpuState::Base, // RepMovsqYqXq
    CpuState::Base, // RepCmpsqXqYq
    CpuState::Base, // RepStosqYqRax
    CpuState::Base, // RepLodsqRaxxq
    CpuState::Base, // RepScasqRaxyq
    CpuState::Base, // CallJq
    CpuState::Base, // JmpJq
    CpuState::Base, // JmpJbq
    CpuState::Base, // JoJq
    CpuState::Base, // JnoJq
    CpuState::Base, // JbJq
    CpuState::Base, // JnbJq
    CpuState::Base, // JzJq
    CpuState::Base, // JnzJq
    CpuState::Base, // JbeJq
    CpuState::Base, // JnbeJq
    CpuState::Base, // JsJq
    CpuState::Base, // JnsJq
    CpuState::Base, // JpJq
    CpuState::Base, // JnpJq
    CpuState::Base, // JlJq
    CpuState::Base, // JnlJq
    CpuState::Base, // JleJq
    CpuState::Base, // JnleJq
    CpuState::Base, // JoJbq
    CpuState::Base, // JnoJbq
    CpuState::Base, // JbJbq
    CpuState::Base, // JnbJbq
    CpuState::Base, // JzJbq
    CpuState::Base, // JnzJbq
    CpuState::Base, // JbeJbq
    CpuState::Base, // JnbeJbq
    CpuState::Base, // JsJbq
    CpuState::Base, // JnsJbq
    CpuState::Base, // JpJbq
    CpuState::Base, // JnpJbq
    CpuState::Base, // JlJbq
    CpuState::Base, // JnlJbq
    CpuState::Base, // JleJbq
    CpuState::Base, // JnleJbq
    CpuState::Base, // EnterOp64IwIb
    CpuState::Base, // LeaveOp64
    CpuState::Base, // IretOp64
    CpuState::Base, // ShldEqGq
    CpuState::Base, // ShldEqGqIb
    CpuState::Base, // ShrdEqGq
    CpuState::Base, // ShrdEqGqIb
    CpuState::Base, // ImulGqEq
    CpuState::Base, // ImulGqEqId
    CpuState::Base, // ImulGqEqsIb
    CpuState::Base, // MovzxGqEb
    CpuState::Base, // MovzxGqEw
    CpuState::Base, // MovsxGqEb
    CpuState::Base, // MovsxGqEw
    CpuState::Base, // MovsxdGqEd
    CpuState::Base, // BswapRrx
    CpuState::Base, // BsfGqEq
    CpuState::Base, // BsrGqEq
    CpuState::Base, // BtEqGq
    CpuState::Base, // BtsEqGq
    CpuState::Base, // BtrEqGq
    CpuState::Base, // BtcEqGq
    CpuState::Base, // BtEqIb
    CpuState::Base, // BtsEqIb
    CpuState::Base, // BtrEqIb
    CpuState::Base, // BtcEqIb
    CpuState::Base, // NotEq
    CpuState::Base, // NegEq
    CpuState::Base, // RolEq
    CpuState::Base, // RorEq
    CpuState::Base, // RclEq
    CpuState::Base, // RcrEq
    CpuState::Base, // ShlEq
    CpuState::Base, // ShrEq
    CpuState::Base, // SarEq
    CpuState::Base, // RolEqIb
    CpuState::Base, // RorEqIb
    CpuState::Base, // RclEqIb
    CpuState::Base, // RcrEqIb
    CpuState::Base, // ShlEqIb
    CpuState::Base, // ShrEqIb
    CpuState::Base, // SarEqIb
    CpuState::Base, // RolEqI1
    CpuState::Base, // RorEqI1
    CpuState::Base, // RclEqI1
    CpuState::Base, // RcrEqI1
    CpuState::Base, // ShlEqI1
    CpuState::Base, // ShrEqI1
    CpuState::Base, // SarEqI1
    CpuState::Base, // MulRaxeq
    CpuState::Base, // ImulRaxeq
    CpuState::Base, // DivRaxeq
    CpuState::Base, // IdivRaxeq
    CpuState::Base, // IncEq
    CpuState::Base, // DecEq
    CpuState::Base, // CallEq
    CpuState::Base, // CallfOp64Ep
    CpuState::Base, // JmpEq
    CpuState::Base, // JmpfOp64Ep
    CpuState::Base, // PushfFq
    CpuState::Base, // PopfFq
    CpuState::Base, // CmpxchgEqGq
    CpuState::Base, // Cdqe
    CpuState::Base, // Cqo
    CpuState::Base, // XaddEqGq
    CpuState::Base, // RetOp64Iw
    CpuState::Base, // RetOp64
    CpuState::Base, // RetfOp64Iw
    CpuState::Base, // RetfOp64
    CpuState::Base, // CmovoGqEq
    CpuState::Base, // CmovnoGqEq
    CpuState::Base, // CmovbGqEq
    CpuState::Base, // CmovnbGqEq
    CpuState::Base, // CmovzGqEq
    CpuState::Base, // CmovnzGqEq
    CpuState::Base, // CmovbeGqEq
    CpuState::Base, // CmovnbeGqEq
    CpuState::Base, // CmovsGqEq
    CpuState::Base, // CmovnsGqEq
    CpuState::Base, // CmovpGqEq
    CpuState::Base, // CmovnpGqEq
    CpuState::Base, // CmovlGqEq
    CpuState::Base, // CmovnlGqEq
    CpuState::Base, // CmovleGqEq
    CpuState::Base, // CmovnleGqEq
    CpuState::Base, // PushEq
    CpuState::Base, // PopEq
    CpuState::Base, // PushOp64Id
    CpuState::Base, // PushOp64SIb
    CpuState::Base, // PushOp64Sw
    CpuState::Base, // PopOp64Sw
    CpuState::Base, // SgdtOp64Ms
    CpuState::Base, // SidtOp64Ms
    CpuState::Base, // LgdtOp64Ms
    CpuState::Base, // LidtOp64Ms
    CpuState::Base, // MovRrxiq
    CpuState::Base, // LssGqMp
    CpuState::Base, // LfsGqMp
    CpuState::Base, // LgsGqMp
    CpuState::Base, // CMPXCHG16B
    CpuState::Base, // LoopneJbq
    CpuState::Base, // LoopeJbq
    CpuState::Base, // LoopJbq
    CpuState::Base, // JrcxzJbq
    CpuState::Sse, // MovqEqVq
    CpuState::Mmx, // MovqPqEq
    CpuState::Sse, // MovqVdqEq
    CpuState::Sse, // Cvtsi2ssVssEq
    CpuState::Sse, // Cvtsi2sdVsdEq
    CpuState::Sse, // Cvttss2siGqWss
    CpuState::Sse, // Cvttsd2siGqWsd
    CpuState::Sse, // Cvtss2siGqWss
    CpuState::Sse, // Cvtsd2siGqWsd
    CpuState::Base, // MovntiOp64MdGd
    CpuState::Base, // MovntiMqGq
    CpuState::Base, // MovCr0rq
    CpuState::Base, // MovCr2rq
    CpuState::Base, // MovCr3rq
    CpuState::Base, // MovCr4rq
    CpuState::Base, // MovRqCr0
    CpuState::Base, // MovRqCr2
    CpuState::Base, // MovRqCr3
    CpuState::Base, // MovRqCr4
    CpuState::Base, // MovDqRq
    CpuState::Base, // MovRqDq
    CpuState::Base, // Swapgs
    CpuState::Base, // RdfsbaseEd
    CpuState::Base, // RdgsbaseEd
    CpuState::Base, // RdfsbaseEq
    CpuState::Base, // RdgsbaseEq
    CpuState::Base, // WrfsbaseEd
    CpuState::Base, // WrgsbaseEd
    CpuState::Base, // WrfsbaseEq
    CpuState::Base, // WrgsbaseEq
    CpuState::Base, // Rdtscp
    CpuState::Base, // VmxonMq
    CpuState::Base, // Vmxoff
    CpuState::Base, // Vmcall
    CpuState::Base, // Vmlaunch
    CpuState::Base, // Vmresume
    CpuState::Base, // VmclearMq
    CpuState::Base, // VmptrldMq
    CpuState::Base, // VmptrstMq
    CpuState::Base, // VmreadEdGd
    CpuState::Base, // VmwriteGdEd
    CpuState::Base, // VmreadEqGq
    CpuState::Base, // VmwriteGqEq
    CpuState::Base, // Invept
    CpuState::Base, // Invvpid
    CpuState::Base, // Vmfunc
    CpuState::Base, // Getsec
    CpuState::Base, // Vmrun
    CpuState::Base, // Vmmcall
    CpuState::Base, // Vmload
    CpuState::Base, // Vmsave
    CpuState::Base, // Stgi
    CpuState::Base, // Clgi
    CpuState::Base, // Skinit
    CpuState::Base, // Invlpga
    CpuState::Base, // Incsspd
    CpuState::Base, // Incsspq
    CpuState::Base, // Rdsspd
    CpuState::Base, // Rdsspq
    CpuState::Base, // Saveprevssp
    CpuState::Base, // Rstorssp
    CpuState::Base, // Wrssd
    CpuState::Base, // Wrussd
    CpuState::Base, // Wrssq
    CpuState::Base, // Wrussq
    CpuState::Base, // Setssbsy
    CpuState::Base, // Clrssbsy
    CpuState::Base, // Endbranch32
    CpuState::Base, // Endbranch64
    CpuState::Base, // Invpcid
    CpuState::Base, // Rdpkru
    CpuState::Base, // Wrpkru
    CpuState::Base, // Clui
    CpuState::Base, // Stui
    CpuState::Base, // Testui
    CpuState::Base, // Uiret
    CpuState::Base, // SenduipiEq
    CpuState::Base, // RdpidEd
    CpuState::Base, // Serialize
    CpuState::Base, // Wrmsrns
    CpuState::Base, // Rdmsrlist
    CpuState::Base, // Wrmsrlist
    CpuState::Avx, // Vzeroupper
    CpuState::Avx, // Vzeroall
    CpuState::Avx, // Vldmxcsr
    CpuState::Avx, // Vstmxcsr
    CpuState::Avx, // VmovapsVpsWps
    CpuState::Avx, // V128VmovapsWpsVps
    CpuState::Avx, // V256VmovapsWpsVps
    CpuState::Avx, // VmovapdVpdWpd
    CpuState::Avx, // V128VmovapdWpdVpd
    CpuState::Avx, // V256VmovapdWpdVpd
    CpuState::Avx, // VmovupsVpsWps
    CpuState::Avx, // V128VmovupsWpsVps
    CpuState::Avx, // V256VmovupsWpsVps
    CpuState::Avx, // VmovupdVpdWpd
    CpuState::Avx, // V128VmovupdWpdVpd
    CpuState::Avx, // V256VmovupdWpdVpd
    CpuState::Avx, // VmovdqaVdqWdq
    CpuState::Avx, // V128VmovdqaWdqVdq
    CpuState::Avx, // V256VmovdqaWdqVdq
    CpuState::Avx, // VmovdquVdqWdq
    CpuState::Avx, // V128VmovdquWdqVdq
    CpuState::Avx, // V256VmovdquWdqVdq
    CpuState::Avx, // V128VmovsdVsdHpdWsd
    CpuState::Avx, // V128VmovssVssHpsWss
    CpuState::Avx, // V128VmovsdWsdHpdVsd
    CpuState::Avx, // V128VmovssWssHpsVss
    CpuState::Avx, // V128VmovsdVsdWsd
    CpuState::Avx, // V128VmovssVssWss
    CpuState::Avx, // V128VmovsdWsdVsd
    CpuState::Avx, // V128VmovssWssVss
    CpuState::Avx, // V128VmovlpsVpsHpsMq
    CpuState::Avx, // V128VmovhlpsVpsHpsWps
    CpuState::Avx, // V128VmovhpsVpsHpsMq
    CpuState::Avx, // V128VmovlhpsVpsHpsWps
    CpuState::Avx, // V128VmovlpsMqVps
    CpuState::Avx, // V128VmovhpsMqVps
    CpuState::Avx, // V128VmovlpdMqVsd
    CpuState::Avx, // V128VmovhpdMqVsd
    CpuState::Avx, // V128VmovlpdVpdHpdMq
    CpuState::Avx, // V128VmovhpdVpdHpdMq
    CpuState::Avx, // V128VmovddupVpdWpd
    CpuState::Avx, // V256VmovddupVpdWpd
    CpuState::Avx, // VmovsldupVpsWps
    CpuState::Avx, // VmovshdupVpsWps
    CpuState::Avx, // VlddquVdqMdq
    CpuState::Avx, // V128VmovntdqaVdqMdq
    CpuState::Avx, // V256VmovntdqaVdqMdq
    CpuState::Avx, // V128VmovntpsMpsVps
    CpuState::Avx, // V256VmovntpsMpsVps
    CpuState::Avx, // V128VmovntpdMpdVpd
    CpuState::Avx, // V256VmovntpdMpdVpd
    CpuState::Avx, // V128VmovntdqMdqVdq
    CpuState::Avx, // V256VmovntdqMdqVdq
    CpuState::Avx, // VucomissVssWss
    CpuState::Avx, // VcomissVssWss
    CpuState::Avx, // VucomisdVsdWsd
    CpuState::Avx, // VcomisdVsdWsd
    CpuState::Avx, // VrsqrtssVssHpsWss
    CpuState::Avx, // VrsqrtpsVpsWps
    CpuState::Avx, // VrcpssVssHpsWss
    CpuState::Avx, // VrcppsVpsWps
    CpuState::Avx, // VandpsVpsHpsWps
    CpuState::Avx, // VandpdVpdHpdWpd
    CpuState::Avx, // VandnpsVpsHpsWps
    CpuState::Avx, // VandnpdVpdHpdWpd
    CpuState::Avx, // VorpsVpsHpsWps
    CpuState::Avx, // VorpdVpdHpdWpd
    CpuState::Avx, // VxorpsVpsHpsWps
    CpuState::Avx, // VxorpdVpdHpdWpd
    CpuState::Avx, // V128VpshufdVdqWdqIb
    CpuState::Avx, // V256VpshufdVdqWdqIb
    CpuState::Avx, // V128VpshufhwVdqWdqIb
    CpuState::Avx, // V256VpshufhwVdqWdqIb
    CpuState::Avx, // V128VpshuflwVdqWdqIb
    CpuState::Avx, // V256VpshuflwVdqWdqIb
    CpuState::Avx, // VhaddpdVpdHpdWpd
    CpuState::Avx, // VhaddpsVpsHpsWps
    CpuState::Avx, // VhsubpdVpdHpdWpd
    CpuState::Avx, // VhsubpsVpsHpsWps
    CpuState::Avx, // VshufpsVpsHpsWpsIb
    CpuState::Avx, // VshufpdVpdHpdWpdIb
    CpuState::Avx, // VaddsubpdVpdHpdWpd
    CpuState::Avx, // VaddsubpsVpsHpsWps
    CpuState::Avx, // VroundpsVpsWpsIb
    CpuState::Avx, // VroundpdVpdWpdIb
    CpuState::Avx, // VroundsdVsdHpdWsdIb
    CpuState::Avx, // VroundssVssHpsWssIb
    CpuState::Avx, // VdppsVpsHpsWpsIb
    CpuState::Avx, // VdppdVpdHpdWpdIb
    CpuState::Avx, // VaddpsVpsHpsWps
    CpuState::Avx, // VaddpdVpdHpdWpd
    CpuState::Avx, // VaddssVssHpsWss
    CpuState::Avx, // VaddsdVsdHpdWsd
    CpuState::Avx, // VmulpsVpsHpsWps
    CpuState::Avx, // VmulpdVpdHpdWpd
    CpuState::Avx, // VmulssVssHpsWss
    CpuState::Avx, // VmulsdVsdHpdWsd
    CpuState::Avx, // VsubpsVpsHpsWps
    CpuState::Avx, // VsubpdVpdHpdWpd
    CpuState::Avx, // VsubssVssHpsWss
    CpuState::Avx, // VsubsdVsdHpdWsd
    CpuState::Avx, // VdivpsVpsHpsWps
    CpuState::Avx, // VdivpdVpdHpdWpd
    CpuState::Avx, // VdivssVssHpsWss
    CpuState::Avx, // VdivsdVsdHpdWsd
    CpuState::Avx, // VmaxpsVpsHpsWps
    CpuState::Avx, // VmaxpdVpdHpdWpd
    CpuState::Avx, // VmaxssVssHpsWss
    CpuState::Avx, // VmaxsdVsdHpdWsd
    CpuState::Avx, // VminpsVpsHpsWps
    CpuState::Avx, // VminpdVpdHpdWpd
    CpuState::Avx, // VminssVssHpsWss
    CpuState::Avx, // VminsdVsdHpdWsd
    CpuState::Avx, // VsqrtpsVpsWps
    CpuState::Avx, // VsqrtpdVpdWpd
    CpuState::Avx, // VsqrtssVssHpsWss
    CpuState::Avx, // VsqrtsdVsdHpdWsd
    CpuState::Avx, // VcmppsVpsHpsWpsIb
    CpuState::Avx, // VcmppdVpdHpdWpdIb
    CpuState::Avx, // VcmpssVssHpsWssIb
    CpuState::Avx, // VcmpsdVsdHpdWsdIb
    CpuState::Avx, // V128VpsrlwVdqHdqWdq
    CpuState::Avx, // V256VpsrlwVdqHdqWdq
    CpuState::Avx, // V128VpsrldVdqHdqWdq
    CpuState::Avx, // V256VpsrldVdqHdqWdq
    CpuState::Avx, // V128VpsrlqVdqHdqWdq
    CpuState::Avx, // V256VpsrlqVdqHdqWdq
    CpuState::Avx, // V128VpsrawVdqHdqWdq
    CpuState::Avx, // V256VpsrawVdqHdqWdq
    CpuState::Avx, // V128VpsradVdqHdqWdq
    CpuState::Avx, // V256VpsradVdqHdqWdq
    CpuState::Avx, // V128VpsllwVdqHdqWdq
    CpuState::Avx, // V256VpsllwVdqHdqWdq
    CpuState::Avx, // V128VpslldVdqHdqWdq
    CpuState::Avx, // V256VpslldVdqHdqWdq
    CpuState::Avx, // V128VpsllqVdqHdqWdq
    CpuState::Avx, // V256VpsllqVdqHdqWdq
    CpuState::Avx, // V128VpsrlwUdqIb
    CpuState::Avx, // V256VpsrlwUdqIb
    CpuState::Avx, // V128VpsrawUdqIb
    CpuState::Avx, // V256VpsrawUdqIb
    CpuState::Avx, // V128VpsllwUdqIb
    CpuState::Avx, // V256VpsllwUdqIb
    CpuState::Avx, // V128VpsrldUdqIb
    CpuState::Avx, // V256VpsrldUdqIb
    CpuState::Avx, // V128VpsradUdqIb
    CpuState::Avx, // V256VpsradUdqIb
    CpuState::Avx, // V128VpslldUdqIb
    CpuState::Avx, // V256VpslldUdqIb
    CpuState::Avx, // V128VpsrlqUdqIb
    CpuState::Avx, // V256VpsrlqUdqIb
    CpuState::Avx, // V128VpsllqUdqIb
    CpuState::Avx, // V256VpsllqUdqIb
    CpuState::Avx, // V128VpsrldqUdqIb
    CpuState::Avx, // V256VpsrldqUdqIb
    CpuState::Avx, // V128VpslldqUdqIb
    CpuState::Avx, // V256VpslldqUdqIb
    CpuState::Avx, // V128VpmovmskbGdUdq
    CpuState::Avx, // V256VpmovmskbGdUdq
    CpuState::Avx, // VmovmskpsGdUps
    CpuState::Avx, // VmovmskpdGdUpd
    CpuState::Avx, // VunpcklpdVpdHpdWpd
    CpuState::Avx, // VunpckhpdVpdHpdWpd
    CpuState::Avx, // VunpcklpsVpsHpsWps
    CpuState::Avx, // VunpckhpsVpsHpsWps
    CpuState::Avx, // V128VpunpckhdqVdqHdqWdq
    CpuState::Avx, // V256VpunpckhdqVdqHdqWdq
    CpuState::Avx, // V128VpunpckldqVdqHdqWdq
    CpuState::Avx, // V256VpunpckldqVdqHdqWdq
    CpuState::Avx, // V128VpunpcklbwVdqHdqWdq
    CpuState::Avx, // V256VpunpcklbwVdqHdqWdq
    CpuState::Avx, // V128VpunpcklwdVdqHdqWdq
    CpuState::Avx, // V256VpunpcklwdVdqHdqWdq
    CpuState::Avx, // V128VpunpckhbwVdqHdqWdq
    CpuState::Avx, // V256VpunpckhbwVdqHdqWdq
    CpuState::Avx, // V128VpunpckhwdVdqHdqWdq
    CpuState::Avx, // V256VpunpckhwdVdqHdqWdq
    CpuState::Avx, // V128VpunpcklqdqVdqHdqWdq
    CpuState::Avx, // V256VpunpcklqdqVdqHdqWdq
    CpuState::Avx, // V128VpunpckhqdqVdqHdqWdq
    CpuState::Avx, // V256VpunpckhqdqVdqHdqWdq
    CpuState::Avx, // V128VpcmpeqbVdqHdqWdq
    CpuState::Avx, // V256VpcmpeqbVdqHdqWdq
    CpuState::Avx, // V128VpcmpeqwVdqHdqWdq
    CpuState::Avx, // V256VpcmpeqwVdqHdqWdq
    CpuState::Avx, // V128VpcmpeqdVdqHdqWdq
    CpuState::Avx, // V256VpcmpeqdVdqHdqWdq
    CpuState::Avx, // V128VpcmpeqqVdqHdqWdq
    CpuState::Avx, // V256VpcmpeqqVdqHdqWdq
    CpuState::Avx, // V128VpcmpgtbVdqHdqWdq
    CpuState::Avx, // V256VpcmpgtbVdqHdqWdq
    CpuState::Avx, // V128VpcmpgtwVdqHdqWdq
    CpuState::Avx, // V256VpcmpgtwVdqHdqWdq
    CpuState::Avx, // V128VpcmpgtdVdqHdqWdq
    CpuState::Avx, // V256VpcmpgtdVdqHdqWdq
    CpuState::Avx, // V128VpcmpgtqVdqHdqWdq
    CpuState::Avx, // V256VpcmpgtqVdqHdqWdq
    CpuState::Avx, // V128VpsubsbVdqHdqWdq
    CpuState::Avx, // V256VpsubsbVdqHdqWdq
    CpuState::Avx, // V128VpsubswVdqHdqWdq
    CpuState::Avx, // V256VpsubswVdqHdqWdq
    CpuState::Avx, // V128VpaddsbVdqHdqWdq
    CpuState::Avx, // V256VpaddsbVdqHdqWdq
    CpuState::Avx, // V128VpaddswVdqHdqWdq
    CpuState::Avx, // V256VpaddswVdqHdqWdq
    CpuState::Avx, // V128VpsubusbVdqHdqWdq
    CpuState::Avx, // V256VpsubusbVdqHdqWdq
    CpuState::Avx, // V128VpsubuswVdqHdqWdq
    CpuState::Avx, // V256VpsubuswVdqHdqWdq
    CpuState::Avx, // V128VpaddusbVdqHdqWdq
    CpuState::Avx, // V256VpaddusbVdqHdqWdq
    CpuState::Avx, // V128VpadduswVdqHdqWdq
    CpuState::Avx, // V256VpadduswVdqHdqWdq
    CpuState::Avx, // V128VpavgbVdqWdq
    CpuState::Avx, // V256VpavgbVdqWdq
    CpuState::Avx, // V128VpavgwVdqWdq
    CpuState::Avx, // V256VpavgwVdqWdq
    CpuState::Avx, // V128VpandnVdqHdqWdq
    CpuState::Avx, // V256VpandnVdqHdqWdq
    CpuState::Avx, // V128VpandVdqHdqWdq
    CpuState::Avx, // V256VpandVdqHdqWdq
    CpuState::Avx, // V128VporVdqHdqWdq
    CpuState::Avx, // V256VporVdqHdqWdq
    CpuState::Avx, // V128VpxorVdqHdqWdq
    CpuState::Avx, // V256VpxorVdqHdqWdq
    CpuState::Avx, // V128VpmulhrswVdqHdqWdq
    CpuState::Avx, // V256VpmulhrswVdqHdqWdq
    CpuState::Avx, // V128VpmuldqVdqHdqWdq
    CpuState::Avx, // V256VpmuldqVdqHdqWdq
    CpuState::Avx, // V128VpmuludqVdqHdqWdq
    CpuState::Avx, // V256VpmuludqVdqHdqWdq
    CpuState::Avx, // V128VpmulldVdqHdqWdq
    CpuState::Avx, // V256VpmulldVdqHdqWdq
    CpuState::Avx, // V128VpmullwVdqHdqWdq
    CpuState::Avx, // V256VpmullwVdqHdqWdq
    CpuState::Avx, // V128VpmulhwVdqHdqWdq
    CpuState::Avx, // V256VpmulhwVdqHdqWdq
    CpuState::Avx, // V128VpmulhuwVdqHdqWdq
    CpuState::Avx, // V256VpmulhuwVdqHdqWdq
    CpuState::Avx, // V128VpsadbwVdqHdqWdq
    CpuState::Avx, // V256VpsadbwVdqHdqWdq
    CpuState::Avx, // V128VmaskmovdquVdqUdq
    CpuState::Avx, // V128VpsubbVdqHdqWdq
    CpuState::Avx, // V256VpsubbVdqHdqWdq
    CpuState::Avx, // V128VpsubwVdqHdqWdq
    CpuState::Avx, // V256VpsubwVdqHdqWdq
    CpuState::Avx, // V128VpsubdVdqHdqWdq
    CpuState::Avx, // V256VpsubdVdqHdqWdq
    CpuState::Avx, // V128VpsubqVdqHdqWdq
    CpuState::Avx, // V256VpsubqVdqHdqWdq
    CpuState::Avx, // V128VpaddbVdqHdqWdq
    CpuState::Avx, // V256VpaddbVdqHdqWdq
    CpuState::Avx, // V128VpaddwVdqHdqWdq
    CpuState::Avx, // V256VpaddwVdqHdqWdq
    CpuState::Avx, // V128VpadddVdqHdqWdq
    CpuState::Avx, // V256VpadddVdqHdqWdq
    CpuState::Avx, // V128VpaddqVdqHdqWdq
    CpuState::Avx, // V256VpaddqVdqHdqWdq
    CpuState::Avx, // V128VpshufbVdqHdqWdq
    CpuState::Avx, // V256VpshufbVdqHdqWdq
    CpuState::Avx, // V128VphaddwVdqHdqWdq
    CpuState::Avx, // V256VphaddwVdqHdqWdq
    CpuState::Avx, // V128VphadddVdqHdqWdq
    CpuState::Avx, // V256VphadddVdqHdqWdq
    CpuState::Avx, // V128VphsubwVdqHdqWdq
    CpuState::Avx, // V256VphsubwVdqHdqWdq
    CpuState::Avx, // V128VphsubdVdqHdqWdq
    CpuState::Avx, // V256VphsubdVdqHdqWdq
    CpuState::Avx, // V128VphaddswVdqHdqWdq
    CpuState::Avx, // V256VphaddswVdqHdqWdq
    CpuState::Avx, // V128VphsubswVdqHdqWdq
    CpuState::Avx, // V256VphsubswVdqHdqWdq
    CpuState::Avx, // V128VpmaddwdVdqHdqWdq
    CpuState::Avx, // V256VpmaddwdVdqHdqWdq
    CpuState::Avx, // V128VpmaddubswVdqHdqWdq
    CpuState::Avx, // V256VpmaddubswVdqHdqWdq
    CpuState::Avx, // V128VpsignbVdqHdqWdq
    CpuState::Avx, // V256VpsignbVdqHdqWdq
    CpuState::Avx, // V128VpsignwVdqHdqWdq
    CpuState::Avx, // V256VpsignwVdqHdqWdq
    CpuState::Avx, // V128VpsigndVdqHdqWdq
    CpuState::Avx, // V256VpsigndVdqHdqWdq
    CpuState::Avx, // VtestpsVpsWps
    CpuState::Avx, // VtestpdVpdWpd
    CpuState::Avx, // VptestVdqWdq
    CpuState::Avx, // VbroadcastssVpsMss
    CpuState::Avx, // V256VbroadcastsdVpdMsd
    CpuState::Avx, // V256Vbroadcastf128VdqMdq
    CpuState::Avx, // V128VpabsbVdqWdq
    CpuState::Avx, // V256VpabsbVdqWdq
    CpuState::Avx, // V128VpabswVdqWdq
    CpuState::Avx, // V256VpabswVdqWdq
    CpuState::Avx, // V128VpabsdVdqWdq
    CpuState::Avx, // V256VpabsdVdqWdq
    CpuState::Avx, // V128VpacksswbVdqHdqWdq
    CpuState::Avx, // V256VpacksswbVdqHdqWdq
    CpuState::Avx, // V128VpackuswbVdqHdqWdq
    CpuState::Avx, // V256VpackuswbVdqHdqWdq
    CpuState::Avx, // V128VpackusdwVdqHdqWdq
    CpuState::Avx, // V256VpackusdwVdqHdqWdq
    CpuState::Avx, // V128VpackssdwVdqHdqWdq
    CpuState::Avx, // V256VpackssdwVdqHdqWdq
    CpuState::Avx, // VmaskmovpsVpsHpsMps
    CpuState::Avx, // VmaskmovpdVpdHpdMpd
    CpuState::Avx, // VmaskmovpsMpsHpsVps
    CpuState::Avx, // VmaskmovpdMpdHpdVpd
    CpuState::Avx, // V128VpmovsxbwVdqWq
    CpuState::Avx, // V128VpmovsxbdVdqWd
    CpuState::Avx, // V128VpmovsxbqVdqWw
    CpuState::Avx, // V128VpmovsxwdVdqWq
    CpuState::Avx, // V128VpmovsxwqVdqWd
    CpuState::Avx, // V128VpmovsxdqVdqWq
    CpuState::Avx, // V128VpmovzxbwVdqWq
    CpuState::Avx, // V128VpmovzxbdVdqWd
    CpuState::Avx, // V128VpmovzxbqVdqWw
    CpuState::Avx, // V128VpmovzxwdVdqWq
    CpuState::Avx, // V128VpmovzxwqVdqWd
    CpuState::Avx, // V128VpmovzxdqVdqWq
    CpuState::Avx, // V128VpminsbVdqHdqWdq
    CpuState::Avx, // V256VpminsbVdqHdqWdq
    CpuState::Avx, // V128VpminswVdqHdqWdq
    CpuState::Avx, // V256VpminswVdqHdqWdq
    CpuState::Avx, // V128VpminsdVdqHdqWdq
    CpuState::Avx, // V256VpminsdVdqHdqWdq
    CpuState::Avx, // V128VpminubVdqHdqWdq
    CpuState::Avx, // V256VpminubVdqHdqWdq
    CpuState::Avx, // V128VpminuwVdqHdqWdq
    CpuState::Avx, // V256VpminuwVdqHdqWdq
    CpuState::Avx, // V128VpminudVdqHdqWdq
    CpuState::Avx, // V256VpminudVdqHdqWdq
    CpuState::Avx, // V128VpmaxsbVdqHdqWdq
    CpuState::Avx, // V256VpmaxsbVdqHdqWdq
    CpuState::Avx, // V128VpmaxswVdqHdqWdq
    CpuState::Avx, // V256VpmaxswVdqHdqWdq
    CpuState::Avx, // V128VpmaxsdVdqHdqWdq
    CpuState::Avx, // V256VpmaxsdVdqHdqWdq
    CpuState::Avx, // V128VpmaxubVdqHdqWdq
    CpuState::Avx, // V256VpmaxubVdqHdqWdq
    CpuState::Avx, // V128VpmaxuwVdqHdqWdq
    CpuState::Avx, // V256VpmaxuwVdqHdqWdq
    CpuState::Avx, // V128VpmaxudVdqHdqWdq
    CpuState::Avx, // V256VpmaxudVdqHdqWdq
    CpuState::Avx, // V128VphminposuwVdqWdq
    CpuState::Avx, // VpermilpsVpsHpsWps
    CpuState::Avx, // VpermilpdVpdHpdWpd
    CpuState::Avx, // VpermilpsVpsWpsIb
    CpuState::Avx, // VpermilpdVpdWpdIb
    CpuState::Avx, // VblendpsVpsHpsWpsIb
    CpuState::Avx, // VblendpdVpdHpdWpdIb
    CpuState::Avx, // V128VpblendwVdqHdqWdqIb
    CpuState::Avx, // V256VpblendwVdqHdqWdqIb
    CpuState::Avx, // V128VpalignrVdqHdqWdqIb
    CpuState::Avx, // V256VpalignrVdqHdqWdqIb
    CpuState::Avx, // V128VinsertpsVpsWssIb
    CpuState::Avx, // V128VextractpsEdVpsIb
    CpuState::Avx, // V256Vperm2f128VdqHdqWdqIb
    CpuState::Avx, // V256Vinsertf128VdqHdqWdqIb
    CpuState::Avx, // V256Vextractf128WdqVdqIb
    CpuState::Avx, // VblendvpsVpsHpsWpsIb
    CpuState::Avx, // VblendvpdVpdHpdWpdIb
    CpuState::Avx, // V128VpblendvbVdqHdqWdqIb
    CpuState::Avx, // V256VpblendvbVdqHdqWdqIb
    CpuState::Avx, // V128VmpsadbwVdqHdqWdqIb
    CpuState::Avx, // V256VmpsadbwVdqHdqWdqIb
    CpuState::Avx, // V128VpcmpestrmVdqWdqIb
    CpuState::Avx, // V128VpcmpestriVdqWdqIb
    CpuState::Avx, // V128VpcmpistrmVdqWdqIb
    CpuState::Avx, // V128VpcmpistriVdqWdqIb
    CpuState::Avx, // V128VaesimcVdqWdq
    CpuState::Avx, // V128VaeskeygenassistVdqWdqIb
    CpuState::Avx, // V128VaesencVdqHdqWdq
    CpuState::Avx, // V128VaesenclastVdqHdqWdq
    CpuState::Avx, // V128VaesdecVdqHdqWdq
    CpuState::Avx, // V128VaesdeclastVdqHdqWdq
    CpuState::Avx, // V128VpclmulqdqVdqHdqWdqIb
    CpuState::Avx, // V256VaesencVdqHdqWdq
    CpuState::Avx, // V256VaesenclastVdqHdqWdq
    CpuState::Avx, // V256VaesdecVdqHdqWdq
    CpuState::Avx, // V256VaesdeclastVdqHdqWdq
    CpuState::Avx, // V256VpclmulqdqVdqHdqWdqIb
    CpuState::Avx, // Vgf2p8affineqbVdqHdqWdqIb
    CpuState::Avx, // Vgf2p8affineinvqbVdqHdqWdqIb
    CpuState::Avx, // Vgf2p8mulbVdqHdqWdq
    CpuState::Avx, // Vsm3msg1VdqHdqWdq
    CpuState::Avx, // Vsm3msg2VdqHdqWdq
    CpuState::Avx, // Vsm3rnds2VdqHdqWdqIb
    CpuState::Avx, // Vsm4key4VdqHdqWdq
    CpuState::Avx, // Vsm4rnds4VdqHdqWdq
    CpuState::Avx, // Vsha512msg1VdqWdq
    CpuState::Avx, // Vsha512msg2VdqWdq
    CpuState::Avx, // Vsha512rnds2VdqHdqWdq
    CpuState::Avx, // V128VmovdVdqEd
    CpuState::Avx, // V128VmovqVdqEq
    CpuState::Avx, // V128VmovdEdVd
    CpuState::Avx, // V128VmovqEqVq
    CpuState::Avx, // V128VpinsrbVdqEbIb
    CpuState::Avx, // V128VpinsrwVdqEwIb
    CpuState::Avx, // V128VpextrwGdUdqIb
    CpuState::Avx, // V128VpextrbEdVdqIbR
    CpuState::Avx, // V128VpextrbMbVdqIbM
    CpuState::Avx, // V128VpextrwEdVdqIbR
    CpuState::Avx, // V128VpextrwMwVdqIbM
    CpuState::Avx, // V128VpinsrdVdqEdIb
    CpuState::Avx, // V128VpinsrqVdqEqIb
    CpuState::Avx, // V128VpextrdEdVdqIb
    CpuState::Avx, // V128VpextrqEqVdqIb
    CpuState::Avx, // Vcvtps2pdVpdWps
    CpuState::Avx, // Vcvttpd2dqVdqWpd
    CpuState::Avx, // Vcvtpd2dqVdqWpd
    CpuState::Avx, // Vcvtdq2pdVpdWdq
    CpuState::Avx, // Vcvtpd2psVpsWpd
    CpuState::Avx, // Vcvtsd2ssVssWsd
    CpuState::Avx, // Vcvtss2sdVsdWss
    CpuState::Avx, // Vcvtdq2psVpsWdq
    CpuState::Avx, // Vcvtps2dqVdqWps
    CpuState::Avx, // Vcvttps2dqVdqWps
    CpuState::Avx, // Vcvtss2siGdWss
    CpuState::Avx, // Vcvtss2siGqWss
    CpuState::Avx, // Vcvtsd2siGdWsd
    CpuState::Avx, // Vcvtsd2siGqWsd
    CpuState::Avx, // Vcvttss2siGdWss
    CpuState::Avx, // Vcvttss2siGqWss
    CpuState::Avx, // Vcvttsd2siGdWsd
    CpuState::Avx, // Vcvttsd2siGqWsd
    CpuState::Avx, // Vcvtsi2ssVssEd
    CpuState::Avx, // Vcvtsi2ssVssEq
    CpuState::Avx, // Vcvtsi2sdVsdEd
    CpuState::Avx, // Vcvtsi2sdVsdEq
    CpuState::Avx, // VmovqWqVq
    CpuState::Avx, // VmovqVqWq
    CpuState::Avx, // Vcvtph2psVpsWps
    CpuState::Avx, // Vcvtps2phWpsVpsIb
    CpuState::Avx, // V256VpmovsxbwVdqWdq
    CpuState::Avx, // V256VpmovsxbdVdqWq
    CpuState::Avx, // V256VpmovsxbqVdqWd
    CpuState::Avx, // V256VpmovsxwdVdqWdq
    CpuState::Avx, // V256VpmovsxwqVdqWq
    CpuState::Avx, // V256VpmovsxdqVdqWdq
    CpuState::Avx, // V256VpmovzxbwVdqWdq
    CpuState::Avx, // V256VpmovzxbdVdqWq
    CpuState::Avx, // V256VpmovzxbqVdqWd
    CpuState::Avx, // V256VpmovzxwdVdqWdq
    CpuState::Avx, // V256VpmovzxwqVdqWq
    CpuState::Avx, // V256VpmovzxdqVdqWdq
    CpuState::Avx, // V256Vperm2i128VdqHdqWdqIb
    CpuState::Avx, // V256Vinserti128VdqHdqWdqIb
    CpuState::Avx, // V256Vextracti128WdqVdqIb
    CpuState::Avx, // V256Vbroadcasti128VdqMdq
    CpuState::Avx, // VpbroadcastbVdqWb
    CpuState::Avx, // VpbroadcastwVdqWw
    CpuState::Avx, // VpbroadcastdVdqWd
    CpuState::Avx, // VpbroadcastqVdqWq
    CpuState::Avx, // VbroadcastssVpsWss
    CpuState::Avx, // V256VbroadcastsdVpdWsd
    CpuState::Avx, // VpblenddVdqHdqWdqIb
    CpuState::Avx, // VmaskmovdVdqHdqMdq
    CpuState::Avx, // VmaskmovqVdqHdqMdq
    CpuState::Avx, // VmaskmovdMdqHdqVdq
    CpuState::Avx, // VmaskmovqMdqHdqVdq
    CpuState::Avx, // VgatherdpsVpsHps
    CpuState::Avx, // VgatherdpdVpdHpd
    CpuState::Avx, // VgatherqpsVpsHps
    CpuState::Avx, // VgatherqpdVpdHpd
    CpuState::Avx, // VgatherddVdqHdq
    CpuState::Avx, // VgatherdqVdqHdq
    CpuState::Avx, // VgatherqdVdqHdq
    CpuState::Avx, // VgatherqqVdqHdq
    CpuState::Avx, // VpsrlvdVdqHdqWdq
    CpuState::Avx, // VpsrlvqVdqHdqWdq
    CpuState::Avx, // VpsllvdVdqHdqWdq
    CpuState::Avx, // VpsllvqVdqHdqWdq
    CpuState::Avx, // V256VpermqVdqWdqIb
    CpuState::Avx, // V256VpermdVdqHdqWdq
    CpuState::Avx, // V256VpermpsVpsHpsWps
    CpuState::Avx, // V256VpermpdVpdWpdIb
    CpuState::Avx, // VpsravdVdqHdqWdq
    CpuState::Avx, // Vfmadd132psVpsHpsWps
    CpuState::Avx, // Vfmadd132pdVpdHpdWpd
    CpuState::Avx, // Vfmadd213psVpsHpsWps
    CpuState::Avx, // Vfmadd213pdVpdHpdWpd
    CpuState::Avx, // Vfmadd231psVpsHpsWps
    CpuState::Avx, // Vfmadd231pdVpdHpdWpd
    CpuState::Avx, // Vfmadd132ssVpsHssWss
    CpuState::Avx, // Vfmadd132sdVpdHsdWsd
    CpuState::Avx, // Vfmadd213ssVpsHssWss
    CpuState::Avx, // Vfmadd213sdVpdHsdWsd
    CpuState::Avx, // Vfmadd231ssVpsHssWss
    CpuState::Avx, // Vfmadd231sdVpdHsdWsd
    CpuState::Avx, // Vfmaddsub132psVpsHpsWps
    CpuState::Avx, // Vfmaddsub132pdVpdHpdWpd
    CpuState::Avx, // Vfmaddsub213psVpsHpsWps
    CpuState::Avx, // Vfmaddsub213pdVpdHpdWpd
    CpuState::Avx, // Vfmaddsub231psVpsHpsWps
    CpuState::Avx, // Vfmaddsub231pdVpdHpdWpd
    CpuState::Avx, // Vfmsubadd132psVpsHpsWps
    CpuState::Avx, // Vfmsubadd132pdVpdHpdWpd
    CpuState::Avx, // Vfmsubadd213psVpsHpsWps
    CpuState::Avx, // Vfmsubadd213pdVpdHpdWpd
    CpuState::Avx, // Vfmsubadd231psVpsHpsWps
    CpuState::Avx, // Vfmsubadd231pdVpdHpdWpd
    CpuState::Avx, // Vfmsub132psVpsHpsWps
    CpuState::Avx, // Vfmsub132pdVpdHpdWpd
    CpuState::Avx, // Vfmsub213psVpsHpsWps
    CpuState::Avx, // Vfmsub213pdVpdHpdWpd
    CpuState::Avx, // Vfmsub231psVpsHpsWps
    CpuState::Avx, // Vfmsub231pdVpdHpdWpd
    CpuState::Avx, // Vfmsub132ssVpsHssWss
    CpuState::Avx, // Vfmsub132sdVpdHsdWsd
    CpuState::Avx, // Vfmsub213ssVpsHssWss
    CpuState::Avx, // Vfmsub213sdVpdHsdWsd
    CpuState::Avx, // Vfmsub231ssVpsHssWss
    CpuState::Avx, // Vfmsub231sdVpdHsdWsd
    CpuState::Avx, // Vfnmadd132psVpsHpsWps
    CpuState::Avx, // Vfnmadd132pdVpdHpdWpd
    CpuState::Avx, // Vfnmadd213psVpsHpsWps
    CpuState::Avx, // Vfnmadd213pdVpdHpdWpd
    CpuState::Avx, // Vfnmadd231psVpsHpsWps
    CpuState::Avx, // Vfnmadd231pdVpdHpdWpd
    CpuState::Avx, // Vfnmadd132ssVpsHssWss
    CpuState::Avx, // Vfnmadd132sdVpdHsdWsd
    CpuState::Avx, // Vfnmadd213ssVpsHssWss
    CpuState::Avx, // Vfnmadd213sdVpdHsdWsd
    CpuState::Avx, // Vfnmadd231ssVpsHssWss
    CpuState::Avx, // Vfnmadd231sdVpdHsdWsd
    CpuState::Avx, // Vfnmsub132psVpsHpsWps
    CpuState::Avx, // Vfnmsub132pdVpdHpdWpd
    CpuState::Avx, // Vfnmsub213psVpsHpsWps
    CpuState::Avx, // Vfnmsub213pdVpdHpdWpd
    CpuState::Avx, // Vfnmsub231psVpsHpsWps
    CpuState::Avx, // Vfnmsub231pdVpdHpdWpd
    CpuState::Avx, // Vfnmsub132ssVpsHssWss
    CpuState::Avx, // Vfnmsub132sdVpdHsdWsd
    CpuState::Avx, // Vfnmsub213ssVpsHssWss
    CpuState::Avx, // Vfnmsub213sdVpdHsdWsd
    CpuState::Avx, // Vfnmsub231ssVpsHssWss
    CpuState::Avx, // Vfnmsub231sdVpdHsdWsd
    CpuState::Avx, // VpdpbusdVdqHdqWdq
    CpuState::Avx, // VpdpbusdsVdqHdqWdq
    CpuState::Avx, // VpdpwssdVdqHdqWdq
    CpuState::Avx, // VpdpwssdsVdqHdqWdq
    CpuState::Avx, // Vpmadd52luqVdqHdqWdq
    CpuState::Avx, // Vpmadd52huqVdqHdqWdq
    CpuState::Avx, // VpdpbssdVdqHdqWdq
    CpuState::Avx, // VpdpbssdsVdqHdqWdq
    CpuState::Avx, // VpdpbsudVdqHdqWdq
    CpuState::Avx, // VpdpbsudsVdqHdqWdq
    CpuState::Avx, // VpdpbuudVdqHdqWdq
    CpuState::Avx, // VpdpbuudsVdqHdqWdq
    CpuState::Avx, // VpdpwsudVdqHdqWdq
    CpuState::Avx, // VpdpwsudsVdqHdqWdq
    CpuState::Avx, // VpdpwusdVdqHdqWdq
    CpuState::Avx, // VpdpwusdsVdqHdqWdq
    CpuState::Avx, // VpdpwuudVdqHdqWdq
    CpuState::Avx, // VpdpwuudsVdqHdqWdq
    CpuState::Avx, // Vbcstnebf162psVpsWw
    CpuState::Avx, // Vbcstnesh2psVpsWsh
    CpuState::Avx, // Vcvtneeph2psVpsWph
    CpuState::Avx, // Vcvtneoph2psVpsWph
    CpuState::Avx, // Vcvtneebf162psVpsWph
    CpuState::Avx, // Vcvtneobf162psVpsWph
    CpuState::Avx, // Vcvtneps2bf16VphWps
    CpuState::Base, // AndnGdBdEd
    CpuState::Base, // AndnGqBqEq
    CpuState::Base, // BlsiBdEd
    CpuState::Base, // BlsiBqEq
    CpuState::Base, // BlsmskBdEd
    CpuState::Base, // BlsmskBqEq
    CpuState::Base, // BlsrBdEd
    CpuState::Base, // BlsrBqEq
    CpuState::Base, // BextrGdEdBd
    CpuState::Base, // BextrGqEqBq
    CpuState::Base, // MulxGdBdEd
    CpuState::Base, // MulxGqBqEq
    CpuState::Base, // RorxGdEdIb
    CpuState::Base, // RorxGqEqIb
    CpuState::Base, // ShlxGdEdBd
    CpuState::Base, // ShlxGqEqBq
    CpuState::Base, // ShrxGdEdBd
    CpuState::Base, // ShrxGqEqBq
    CpuState::Base, // SarxGdEdBd
    CpuState::Base, // SarxGqEqBq
    CpuState::Base, // BzhiGdBdEd
    CpuState::Base, // BzhiGqBqEq
    CpuState::Base, // PextGdBdEd
    CpuState::Base, // PextGqBqEq
    CpuState::Base, // PdepGdBdEd
    CpuState::Base, // PdepGqBqEq
    CpuState::Base, // CmpbexaddEdGdBd
    CpuState::Base, // CmpbexaddEqGqBq
    CpuState::Base, // CmpbxaddEdGdBd
    CpuState::Base, // CmpbxaddEqGqBq
    CpuState::Base, // CmplexaddEdGdBd
    CpuState::Base, // CmplexaddEqGqBq
    CpuState::Base, // CmplxaddEdGdBd
    CpuState::Base, // CmplxaddEqGqBq
    CpuState::Base, // CmpnbexaddEdGdBd
    CpuState::Base, // CmpnbexaddEqGqBq
    CpuState::Base, // CmpnbxaddEdGdBd
    CpuState::Base, // CmpnbxaddEqGqBq
    CpuState::Base, // CmpnlexaddEdGdBd
    CpuState::Base, // CmpnlexaddEqGqBq
    CpuState::Base, // CmpnlxaddEdGdBd
    CpuState::Base, // CmpnlxaddEqGqBq
    CpuState::Base, // CmpnoxaddEdGdBd
    CpuState::Base, // CmpnoxaddEqGqBq
    CpuState::Base, // CmpnpxaddEdGdBd
    CpuState::Base, // CmpnpxaddEqGqBq
    CpuState::Base, // CmpnsxaddEdGdBd
    CpuState::Base, // CmpnsxaddEqGqBq
    CpuState::Base, // CmpnzxaddEdGdBd
    CpuState::Base, // CmpnzxaddEqGqBq
    CpuState::Base, // CmpoxaddEdGdBd
    CpuState::Base, // CmpoxaddEqGqBq
    CpuState::Base, // CmppxaddEdGdBd
    CpuState::Base, // CmppxaddEqGqBq
    CpuState::Base, // CmpsxaddEdGdBd
    CpuState::Base, // CmpsxaddEqGqBq
    CpuState::Base, // CmpzxaddEdGdBd
    CpuState::Base, // CmpzxaddEqGqBq
    CpuState::Avx, // VfmaddsubpsVpsHpsVibWps
    CpuState::Avx, // VfmaddsubpsVpsHpsWpsVib
    CpuState::Avx, // VfmaddsubpdVpdHpdVibWpd
    CpuState::Avx, // VfmaddsubpdVpdHpdWpdVib
    CpuState::Avx, // VfmsubaddpsVpsHpsVibWps
    CpuState::Avx, // VfmsubaddpsVpsHpsWpsVib
    CpuState::Avx, // VfmsubaddpdVpdHpdVibWpd
    CpuState::Avx, // VfmsubaddpdVpdHpdWpdVib
    CpuState::Avx, // VfmaddpsVpsHpsVibWps
    CpuState::Avx, // VfmaddpsVpsHpsWpsVib
    CpuState::Avx, // VfmaddpdVpdHpdVibWpd
    CpuState::Avx, // VfmaddpdVpdHpdWpdVib
    CpuState::Avx, // VfmaddssVssHssVibWss
    CpuState::Avx, // VfmaddssVssHssWssVib
    CpuState::Avx, // VfmaddsdVsdHsdVibWsd
    CpuState::Avx, // VfmaddsdVsdHsdWsdVib
    CpuState::Avx, // VfmsubpsVpsHpsVibWps
    CpuState::Avx, // VfmsubpsVpsHpsWpsVib
    CpuState::Avx, // VfmsubpdVpdHpdVibWpd
    CpuState::Avx, // VfmsubpdVpdHpdWpdVib
    CpuState::Avx, // VfmsubssVssHssVibWss
    CpuState::Avx, // VfmsubssVssHssWssVib
    CpuState::Avx, // VfmsubsdVsdHsdVibWsd
    CpuState::Avx, // VfmsubsdVsdHsdWsdVib
    CpuState::Avx, // VfnmaddpsVpsHpsVibWps
    CpuState::Avx, // VfnmaddpsVpsHpsWpsVib
    CpuState::Avx, // VfnmaddpdVpdHpdVibWpd
    CpuState::Avx, // VfnmaddpdVpdHpdWpdVib
    CpuState::Avx, // VfnmaddssVssHssVibWss
    CpuState::Avx, // VfnmaddssVssHssWssVib
    CpuState::Avx, // VfnmaddsdVsdHsdVibWsd
    CpuState::Avx, // VfnmaddsdVsdHsdWsdVib
    CpuState::Avx, // VfnmsubpsVpsHpsVibWps
    CpuState::Avx, // VfnmsubpsVpsHpsWpsVib
    CpuState::Avx, // VfnmsubpdVpdHpdVibWpd
    CpuState::Avx, // VfnmsubpdVpdHpdWpdVib
    CpuState::Avx, // VfnmsubssVssHssVibWss
    CpuState::Avx, // VfnmsubssVssHssWssVib
    CpuState::Avx, // VfnmsubsdVsdHsdVibWsd
    CpuState::Avx, // VfnmsubsdVsdHsdWsdVib
    CpuState::Avx, // VpcmovVdqHdqVibWdq
    CpuState::Avx, // VpcmovVdqHdqWdqVib
    CpuState::Avx, // VppermVdqHdqVibWdq
    CpuState::Avx, // VppermVdqHdqWdqVib
    CpuState::Avx, // Vpermil2psVdqHdqVibWdq
    CpuState::Avx, // Vpermil2psVdqHdqWdqVib
    CpuState::Avx, // Vpermil2pdVdqHdqVibWdq
    CpuState::Avx, // Vpermil2pdVdqHdqWdqVib
    CpuState::Avx, // VpshabVdqHdqWdq
    CpuState::Avx, // VpshabVdqWdqHdq
    CpuState::Avx, // VpshawVdqHdqWdq
    CpuState::Avx, // VpshawVdqWdqHdq
    CpuState::Avx, // VpshadVdqHdqWdq
    CpuState::Avx, // VpshadVdqWdqHdq
    CpuState::Avx, // VpshaqVdqHdqWdq
    CpuState::Avx, // VpshaqVdqWdqHdq
    CpuState::Avx, // VprotbVdqHdqWdq
    CpuState::Avx, // VprotbVdqWdqHdq
    CpuState::Avx, // VprotwVdqHdqWdq
    CpuState::Avx, // VprotwVdqWdqHdq
    CpuState::Avx, // VprotdVdqHdqWdq
    CpuState::Avx, // VprotdVdqWdqHdq
    CpuState::Avx, // VprotqVdqHdqWdq
    CpuState::Avx, // VprotqVdqWdqHdq
    CpuState::Avx, // VpshlbVdqHdqWdq
    CpuState::Avx, // VpshlbVdqWdqHdq
    CpuState::Avx, // VpshlwVdqHdqWdq
    CpuState::Avx, // VpshlwVdqWdqHdq
    CpuState::Avx, // VpshldVdqHdqWdq
    CpuState::Avx, // VpshldVdqWdqHdq
    CpuState::Avx, // VpshlqVdqHdqWdq
    CpuState::Avx, // VpshlqVdqWdqHdq
    CpuState::Avx, // VpmacsswwVdqHdqWdqVib
    CpuState::Avx, // VpmacsswdVdqHdqWdqVib
    CpuState::Avx, // VpmacssdqlVdqHdqWdqVib
    CpuState::Avx, // VpmacssddVdqHdqWdqVib
    CpuState::Avx, // VpmacssdqhVdqHdqWdqVib
    CpuState::Avx, // VpmacswwVdqHdqWdqVib
    CpuState::Avx, // VpmacswdVdqHdqWdqVib
    CpuState::Avx, // VpmacsdqlVdqHdqWdqVib
    CpuState::Avx, // VpmacsddVdqHdqWdqVib
    CpuState::Avx, // VpmacsdqhVdqHdqWdqVib
    CpuState::Avx, // VpmadcsswdVdqHdqWdqVib
    CpuState::Avx, // VpmadcswdVdqHdqWdqVib
    CpuState::Avx, // VprotbVdqWdqIb
    CpuState::Avx, // VprotwVdqWdqIb
    CpuState::Avx, // VprotdVdqWdqIb
    CpuState::Avx, // VprotqVdqWdqIb
    CpuState::Avx, // VpcombVdqHdqWdqIb
    CpuState::Avx, // VpcomwVdqHdqWdqIb
    CpuState::Avx, // VpcomdVdqHdqWdqIb
    CpuState::Avx, // VpcomqVdqHdqWdqIb
    CpuState::Avx, // VpcomubVdqHdqWdqIb
    CpuState::Avx, // VpcomuwVdqHdqWdqIb
    CpuState::Avx, // VpcomudVdqHdqWdqIb
    CpuState::Avx, // VpcomuqVdqHdqWdqIb
    CpuState::Avx, // VfrczpsVpsWps
    CpuState::Avx, // VfrczpdVpdWpd
    CpuState::Avx, // VfrczssVssWss
    CpuState::Avx, // VfrczsdVsdWsd
    CpuState::Avx, // VphaddbwVdqWdq
    CpuState::Avx, // VphaddbdVdqWdq
    CpuState::Avx, // VphaddbqVdqWdq
    CpuState::Avx, // VphaddwdVdqWdq
    CpuState::Avx, // VphaddwqVdqWdq
    CpuState::Avx, // VphadddqVdqWdq
    CpuState::Avx, // VphaddubwVdqWdq
    CpuState::Avx, // VphaddubdVdqWdq
    CpuState::Avx, // VphaddubqVdqWdq
    CpuState::Avx, // VphadduwdVdqWdq
    CpuState::Avx, // VphadduwqVdqWdq
    CpuState::Avx, // VphaddudqVdqWdq
    CpuState::Avx, // VphsubbwVdqWdq
    CpuState::Avx, // VphsubwdVdqWdq
    CpuState::Avx, // VphsubdqVdqWdq
    CpuState::Base, // BextrGdEdId
    CpuState::Base, // BextrGqEqId
    CpuState::Base, // BlcfillBdEd
    CpuState::Base, // BlcfillBqEq
    CpuState::Base, // BlciBdEd
    CpuState::Base, // BlciBqEq
    CpuState::Base, // BlcicBdEd
    CpuState::Base, // BlcicBqEq
    CpuState::Base, // BlcmskBdEd
    CpuState::Base, // BlcmskBqEq
    CpuState::Base, // BlcsBdEd
    CpuState::Base, // BlcsBqEq
    CpuState::Base, // BlsfillBdEd
    CpuState::Base, // BlsfillBqEq
    CpuState::Base, // BlsicBdEd
    CpuState::Base, // BlsicBqEq
    CpuState::Base, // T1mskcBdEd
    CpuState::Base, // T1mskcBqEq
    CpuState::Base, // TzmskBdEd
    CpuState::Base, // TzmskBqEq
    CpuState::Base, // TzcntGwEw
    CpuState::Base, // TzcntGdEd
    CpuState::Base, // TzcntGqEq
    CpuState::Base, // LzcntGwEw
    CpuState::Base, // LzcntGdEd
    CpuState::Base, // LzcntGqEq
    CpuState::Sse, // MovntssMssVss
    CpuState::Sse, // MovntsdMsdVsd
    CpuState::Sse, // ExtrqUdqIbIb
    CpuState::Sse, // ExtrqVdqUq
    CpuState::Sse, // InsertqVdqUqIbIb
    CpuState::Sse, // InsertqVdqUdq
    CpuState::Base, // AdcxGdEd
    CpuState::Base, // AdoxGdEd
    CpuState::Base, // AdcxGqEq
    CpuState::Base, // AdoxGqEq
    CpuState::Base, // Stac
    CpuState::Base, // Clac
    CpuState::Base, // RdrandEw
    CpuState::Base, // RdrandEd
    CpuState::Base, // RdrandEq
    CpuState::Base, // RdseedEw
    CpuState::Base, // RdseedEd
    CpuState::Base, // RdseedEq
    CpuState::Base, // MovdiriMdGd
    CpuState::Base, // MovdiriMqGq
    CpuState::Base, // Movdir64bGdMdq
    CpuState::Base, // Movdir64bGqMdq
    CpuState::Base, // AaddEdGd
    CpuState::Base, // AandEdGd
    CpuState::Base, // AorEdGd
    CpuState::Base, // AxorEdGd
    CpuState::Base, // AaddEqGq
    CpuState::Base, // AandEqGq
    CpuState::Base, // AorEqGq
    CpuState::Base, // AxorEqGq
    CpuState::Amx, // Ldtilecfg
    CpuState::Amx, // Sttilecfg
    CpuState::Amx, // TileloaddTnnnMdq
    CpuState::Amx, // Tileloaddt1TnnnMdq
    CpuState::Amx, // TileloaddrsTnnnMdq
    CpuState::Amx, // Tileloaddrst1TnnnMdq
    CpuState::Amx, // TilestoredMdqTnnn
    CpuState::Amx, // Tilerelease
    CpuState::Amx, // TilezeroTnnn
    CpuState::Amx, // TdpbssdTnnnTrmTreg
    CpuState::Amx, // TdpbsudTnnnTrmTreg
    CpuState::Amx, // TdpbusdTnnnTrmTreg
    CpuState::Amx, // TdpbuudTnnnTrmTreg
    CpuState::Amx, // Tdpbf16psTnnnTrmTreg
    CpuState::Amx, // Tdpfp16psTnnnTrmTreg
    CpuState::Amx, // Tcmmrlfp16psTnnnTrmTreg
    CpuState::Amx, // Tcmmimfp16psTnnnTrmTreg
    CpuState::Base, // Tmmultf32psTnnnTrmTreg
    CpuState::Amx, // Tdpbf8psTnnnTrmTreg
    CpuState::Amx, // Tdphf8psTnnnTrmTreg
    CpuState::Amx, // Tdpbhf8psTnnnTrmTreg
    CpuState::Amx, // Tdphbf8psTnnnTrmTreg
    CpuState::Evex, // KaddwKgwKhwKew
    CpuState::Evex, // KaddqKgqKhqKeq
    CpuState::Evex, // KaddbKgbKhbKeb
    CpuState::Evex, // KadddKgdKhdKed
    CpuState::Evex, // KandwKgwKhwKew
    CpuState::Evex, // KandqKgqKhqKeq
    CpuState::Evex, // KandbKgbKhbKeb
    CpuState::Evex, // KanddKgdKhdKed
    CpuState::Evex, // KandnwKgwKhwKew
    CpuState::Evex, // KandnqKgqKhqKeq
    CpuState::Evex, // KandnbKgbKhbKeb
    CpuState::Evex, // KandndKgdKhdKed
    CpuState::Evex, // KmovwKgwKew
    CpuState::Evex, // KmovqKgqKeq
    CpuState::Evex, // KmovbKgbKeb
    CpuState::Evex, // KmovdKgdKed
    CpuState::Evex, // KmovwKewKgw
    CpuState::Evex, // KmovqKeqKgq
    CpuState::Evex, // KmovbKebKgb
    CpuState::Evex, // KmovdKedKgd
    CpuState::Evex, // KmovbGdKeb
    CpuState::Evex, // KmovwGdKew
    CpuState::Evex, // KmovdGdKed
    CpuState::Evex, // KmovqGqKeq
    CpuState::Evex, // KmovbKgbEb
    CpuState::Evex, // KmovwKgwEw
    CpuState::Evex, // KmovdKgdEd
    CpuState::Evex, // KmovqKgqEq
    CpuState::Evex, // KunpckbwKgwKhbKeb
    CpuState::Evex, // KunpckwdKgdKhwKew
    CpuState::Evex, // KunpckdqKgqKhdKed
    CpuState::Evex, // KnotwKgwKew
    CpuState::Evex, // KnotqKgqKeq
    CpuState::Evex, // KnotbKgbKeb
    CpuState::Evex, // KnotdKgdKed
    CpuState::Evex, // KorwKgwKhwKew
    CpuState::Evex, // KorqKgqKhqKeq
    CpuState::Evex, // KorbKgbKhbKeb
    CpuState::Evex, // KordKgdKhdKed
    CpuState::Evex, // KortestwKgwKew
    CpuState::Evex, // KortestqKgqKeq
    CpuState::Evex, // KortestbKgbKeb
    CpuState::Evex, // KortestdKgdKed
    CpuState::Evex, // KshiftlbKgbKebIb
    CpuState::Evex, // KshiftlwKgwKewIb
    CpuState::Evex, // KshiftldKgdKedIb
    CpuState::Evex, // KshiftlqKgqKeqIb
    CpuState::Evex, // KshiftrbKgbKebIb
    CpuState::Evex, // KshiftrwKgwKewIb
    CpuState::Evex, // KshiftrdKgdKedIb
    CpuState::Evex, // KshiftrqKgqKeqIb
    CpuState::Evex, // KxnorwKgwKhwKew
    CpuState::Evex, // KxnorqKgqKhqKeq
    CpuState::Evex, // KxnorbKgbKhbKeb
    CpuState::Evex, // KxnordKgdKhdKed
    CpuState::Evex, // KxorwKgwKhwKew
    CpuState::Evex, // KxorqKgqKhqKeq
    CpuState::Evex, // KxorbKgbKhbKeb
    CpuState::Evex, // KxordKgdKhdKed
    CpuState::Evex, // KtestwKgwKew
    CpuState::Evex, // KtestqKgqKeq
    CpuState::Evex, // KtestbKgbKeb
    CpuState::Evex, // KtestdKgdKed
    CpuState::Base, // RdmsrEqId
    CpuState::Base, // WrmsrnsIdEq
    CpuState::Base, // MovrsGbEb
    CpuState::Base, // MovrsGwEw
    CpuState::Base, // MovrsGdEd
    CpuState::Base, // MovrsGqEq
    CpuState::Base, // Erets
    CpuState::Base, // Eretu
    CpuState::Base, // LkgsEw
    CpuState::Evex, // EvexVaddpsVpsHpsWps
    CpuState::Evex, // EvexVaddpdVpdHpdWpd
    CpuState::Evex, // EvexVaddssVssHpsWss
    CpuState::Evex, // EvexVaddsdVsdHpdWsd
    CpuState::Evex, // EvexVaddpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVaddpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVaddssVssHpsWssKmask
    CpuState::Evex, // EvexVaddsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVsubpsVpsHpsWps
    CpuState::Evex, // EvexVsubpdVpdHpdWpd
    CpuState::Evex, // EvexVsubssVssHpsWss
    CpuState::Evex, // EvexVsubsdVsdHpdWsd
    CpuState::Evex, // EvexVsubpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVsubpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVsubssVssHpsWssKmask
    CpuState::Evex, // EvexVsubsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVmulpsVpsHpsWps
    CpuState::Evex, // EvexVmulpdVpdHpdWpd
    CpuState::Evex, // EvexVmulssVssHpsWss
    CpuState::Evex, // EvexVmulsdVsdHpdWsd
    CpuState::Evex, // EvexVmulpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVmulpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVmulssVssHpsWssKmask
    CpuState::Evex, // EvexVmulsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVdivpsVpsHpsWps
    CpuState::Evex, // EvexVdivpdVpdHpdWpd
    CpuState::Evex, // EvexVdivssVssHpsWss
    CpuState::Evex, // EvexVdivsdVsdHpdWsd
    CpuState::Evex, // EvexVdivpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVdivpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVdivssVssHpsWssKmask
    CpuState::Evex, // EvexVdivsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVminpsVpsHpsWps
    CpuState::Evex, // EvexVminpdVpdHpdWpd
    CpuState::Evex, // EvexVminssVssHpsWss
    CpuState::Evex, // EvexVminsdVsdHpdWsd
    CpuState::Evex, // EvexVminpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVminpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVminssVssHpsWssKmask
    CpuState::Evex, // EvexVminsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVmaxpsVpsHpsWps
    CpuState::Evex, // EvexVmaxpdVpdHpdWpd
    CpuState::Evex, // EvexVmaxssVssHpsWss
    CpuState::Evex, // EvexVmaxsdVsdHpdWsd
    CpuState::Evex, // EvexVmaxpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVmaxpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVmaxssVssHpsWssKmask
    CpuState::Evex, // EvexVmaxsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVsqrtpsVpsWps
    CpuState::Evex, // EvexVsqrtpdVpdWpd
    CpuState::Evex, // EvexVsqrtssVssHpsWss
    CpuState::Evex, // EvexVsqrtsdVsdHpdWsd
    CpuState::Evex, // EvexVsqrtpsVpsWpsKmask
    CpuState::Evex, // EvexVsqrtpdVpdWpdKmask
    CpuState::Evex, // EvexVsqrtssVssHpsWssKmask
    CpuState::Evex, // EvexVsqrtsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVcmppsKgwHpsWpsIb
    CpuState::Evex, // EvexVcmppdKgbHpdWpdIb
    CpuState::Evex, // EvexVcmpssKgbHssWssIb
    CpuState::Evex, // EvexVcmpsdKgbHsdWsdIb
    CpuState::Evex, // EvexVrndscalepsVpsWpsIbKmask
    CpuState::Evex, // EvexVrndscalepdVpdWpdIbKmask
    CpuState::Evex, // EvexVrndscalessVssHpsWssIbKmask
    CpuState::Evex, // EvexVrndscalesdVsdHpdWsdIbKmask
    CpuState::Evex, // EvexVunpcklpsVpsHpsWps
    CpuState::Evex, // EvexVunpcklpdVpdHpdWpd
    CpuState::Evex, // EvexVunpcklpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVunpcklpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVunpckhpsVpsHpsWps
    CpuState::Evex, // EvexVunpckhpdVpdHpdWpd
    CpuState::Evex, // EvexVunpckhpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVunpckhpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVpunpckldqVdqHdqWdq
    CpuState::Evex, // EvexVpunpcklqdqVdqHdqWdq
    CpuState::Evex, // EvexVpunpckldqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpcklqdqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpckhdqVdqHdqWdq
    CpuState::Evex, // EvexVpunpckhqdqVdqHdqWdq
    CpuState::Evex, // EvexVpunpckhdqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpckhqdqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmuldqVdqHdqWdq
    CpuState::Evex, // EvexVpmuludqVdqHdqWdq
    CpuState::Evex, // EvexVpmuldqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmuludqVdqHdqWdqKmask
    CpuState::Evex, // EvexVucomissVssWss
    CpuState::Evex, // EvexVcomissVssWss
    CpuState::Evex, // EvexVucomisdVsdWsd
    CpuState::Evex, // EvexVcomisdVsdWsd
    CpuState::Evex, // EvexVcvtss2sdVsdWss
    CpuState::Evex, // EvexVcvtsd2ssVssWsd
    CpuState::Evex, // EvexVcvtps2pdVpdWps
    CpuState::Evex, // EvexVcvtpd2psVpsWpd
    CpuState::Evex, // EvexVcvtss2sdVsdWssKmask
    CpuState::Evex, // EvexVcvtsd2ssVssWsdKmask
    CpuState::Evex, // EvexVcvtps2pdVpdWpsKmask
    CpuState::Evex, // EvexVcvtpd2psVpsWpdKmask
    CpuState::Evex, // EvexVcvtps2dqVdqWps
    CpuState::Evex, // EvexVcvtps2dqVdqWpsKmask
    CpuState::Evex, // EvexVcvttps2dqVdqWps
    CpuState::Evex, // EvexVcvttps2dqVdqWpsKmask
    CpuState::Evex, // EvexVcvtpd2dqVdqWpd
    CpuState::Evex, // EvexVcvtpd2dqVdqWpdKmask
    CpuState::Evex, // EvexVcvttpd2dqVdqWpd
    CpuState::Evex, // EvexVcvttpd2dqVdqWpdKmask
    CpuState::Evex, // EvexVcvtph2psVpsWps
    CpuState::Evex, // EvexVcvtph2psVpsWpsKmask
    CpuState::Evex, // EvexVcvtps2phWpsVpsIb
    CpuState::Evex, // EvexVcvtps2phWpsVpsIbKmask
    CpuState::Evex, // EvexVcvtneps2bf16VphWpsKmask
    CpuState::Evex, // EvexVcvtne2ps2bf16VphHpsWpsKmask
    CpuState::Evex, // EvexVdpbf16psVpsHdqWdqKmask
    CpuState::Evex, // EvexVmovapsVpsWps
    CpuState::Evex, // EvexVmovapsVpsWpsKmask
    CpuState::Evex, // EvexVmovapsWpsVps
    CpuState::Evex, // EvexVmovapsWpsVpsKmask
    CpuState::Evex, // EvexVmovapdVpdWpd
    CpuState::Evex, // EvexVmovapdVpdWpdKmask
    CpuState::Evex, // EvexVmovapdWpdVpd
    CpuState::Evex, // EvexVmovapdWpdVpdKmask
    CpuState::Evex, // EvexVmovupsVpsWps
    CpuState::Evex, // EvexVmovupsVpsWpsKmask
    CpuState::Evex, // EvexVmovupsWpsVps
    CpuState::Evex, // EvexVmovupsWpsVpsKmask
    CpuState::Evex, // EvexVmovupdVpdWpd
    CpuState::Evex, // EvexVmovupdVpdWpdKmask
    CpuState::Evex, // EvexVmovupdWpdVpd
    CpuState::Evex, // EvexVmovupdWpdVpdKmask
    CpuState::Evex, // EvexVmovsdVsdHpdWsd
    CpuState::Evex, // EvexVmovssVssHpsWss
    CpuState::Evex, // EvexVmovsdWsdHpdVsd
    CpuState::Evex, // EvexVmovssWssHpsVss
    CpuState::Evex, // EvexVmovsdVsdWsd
    CpuState::Evex, // EvexVmovssVssWss
    CpuState::Evex, // EvexVmovsdWsdVsd
    CpuState::Evex, // EvexVmovssWssVss
    CpuState::Evex, // EvexVmovsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVmovssVssHpsWssKmask
    CpuState::Evex, // EvexVmovsdWsdHpdVsdKmask
    CpuState::Evex, // EvexVmovssWssHpsVssKmask
    CpuState::Evex, // EvexVmovsdVsdWsdKmask
    CpuState::Evex, // EvexVmovssVssWssKmask
    CpuState::Evex, // EvexVmovsdWsdVsdKmask
    CpuState::Evex, // EvexVmovssWssVssKmask
    CpuState::Evex, // EvexVpabsbVdqWdq
    CpuState::Evex, // EvexVpabswVdqWdq
    CpuState::Evex, // EvexVpabsdVdqWdq
    CpuState::Evex, // EvexVpabsqVdqWdq
    CpuState::Evex, // EvexVpabsbVdqWdqKmask
    CpuState::Evex, // EvexVpabswVdqWdqKmask
    CpuState::Evex, // EvexVpabsdVdqWdqKmask
    CpuState::Evex, // EvexVpabsqVdqWdqKmask
    CpuState::Evex, // EvexVmovntdqaVdqMdq
    CpuState::Evex, // EvexVmovntpsMpsVps
    CpuState::Evex, // EvexVmovntpdMpdVpd
    CpuState::Evex, // EvexVmovntdqMdqVdq
    CpuState::Evex, // EvexVpcmpeqbKgqHdqWdq
    CpuState::Evex, // EvexVpcmpeqwKgdHdqWdq
    CpuState::Evex, // EvexVpcmpgtbKgqHdqWdq
    CpuState::Evex, // EvexVpcmpgtwKgdHdqWdq
    CpuState::Evex, // EvexVpcmpeqdKgwHdqWdq
    CpuState::Evex, // EvexVpcmpeqqKgbHdqWdq
    CpuState::Evex, // EvexVpcmpgtdKgwHdqWdq
    CpuState::Evex, // EvexVpcmpgtqKgbHdqWdq
    CpuState::Evex, // EvexVpsrlwVdqHdqWdq
    CpuState::Evex, // EvexVpsrlwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrawVdqHdqWdq
    CpuState::Evex, // EvexVpsrawVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsllwVdqHdqWdq
    CpuState::Evex, // EvexVpsllwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrlwUdqIb
    CpuState::Evex, // EvexVpsrlwUdqIbKmask
    CpuState::Evex, // EvexVpsllwUdqIb
    CpuState::Evex, // EvexVpsllwUdqIbKmask
    CpuState::Evex, // EvexVpsrawUdqIb
    CpuState::Evex, // EvexVpsrawUdqIbKmask
    CpuState::Evex, // EvexVpsrldVdqHdqWdq
    CpuState::Evex, // EvexVpsrlqVdqHdqWdq
    CpuState::Evex, // EvexVpsrldVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrlqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpslldVdqHdqWdq
    CpuState::Evex, // EvexVpsllqVdqHdqWdq
    CpuState::Evex, // EvexVpslldVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsllqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrldUdqIb
    CpuState::Evex, // EvexVpsrldUdqIbKmask
    CpuState::Evex, // EvexVpsrlqUdqIb
    CpuState::Evex, // EvexVpsrlqUdqIbKmask
    CpuState::Evex, // EvexVpslldUdqIb
    CpuState::Evex, // EvexVpslldUdqIbKmask
    CpuState::Evex, // EvexVpsllqUdqIb
    CpuState::Evex, // EvexVpsllqUdqIbKmask
    CpuState::Evex, // EvexVpshufbVdqHdqWdq
    CpuState::Evex, // EvexVpshufbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermqVdqWdqIbKmask
    CpuState::Evex, // EvexVpermpdVpdWpdIbKmask
    CpuState::Evex, // EvexVshufpsVpsHpsWpsIb
    CpuState::Evex, // EvexVshufpdVpdHpdWpdIb
    CpuState::Evex, // EvexVshufpsVpsHpsWpsIbKmask
    CpuState::Evex, // EvexVshufpdVpdHpdWpdIbKmask
    CpuState::Evex, // EvexVpermilpsVpsHpsWps
    CpuState::Evex, // EvexVpermilpdVpdHpdWpd
    CpuState::Evex, // EvexVpermilpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVpermilpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVpermilpsVpsWpsIb
    CpuState::Evex, // EvexVpermilpdVpdWpdIb
    CpuState::Evex, // EvexVpermilpsVpsWpsIbKmask
    CpuState::Evex, // EvexVpermilpdVpdWpdIbKmask
    CpuState::Evex, // EvexVpshufdVdqWdqIb
    CpuState::Evex, // EvexVpshufdVdqWdqIbKmask
    CpuState::Evex, // EvexVpshuflwVdqWdqIb
    CpuState::Evex, // EvexVpshuflwVdqWdqIbKmask
    CpuState::Evex, // EvexVpshufhwVdqWdqIb
    CpuState::Evex, // EvexVpshufhwVdqWdqIbKmask
    CpuState::Evex, // EvexVpbroadcastbVdqEb
    CpuState::Evex, // EvexVpbroadcastbVdqEbKmask
    CpuState::Evex, // EvexVpbroadcastwVdqEw
    CpuState::Evex, // EvexVpbroadcastwVdqEwKmask
    CpuState::Evex, // EvexVpbroadcastdVdqEd
    CpuState::Evex, // EvexVpbroadcastdVdqEdKmask
    CpuState::Evex, // EvexVpbroadcastqVdqEq
    CpuState::Evex, // EvexVpbroadcastqVdqEqKmask
    CpuState::Evex, // EvexVpbroadcastbVdqWb
    CpuState::Evex, // EvexVpbroadcastbVdqWbKmask
    CpuState::Evex, // EvexVpbroadcastwVdqWw
    CpuState::Evex, // EvexVpbroadcastwVdqWwKmask
    CpuState::Evex, // EvexVpbroadcastdVdqWd
    CpuState::Evex, // EvexVpbroadcastdVdqWdKmask
    CpuState::Evex, // EvexVpbroadcastqVdqWq
    CpuState::Evex, // EvexVpbroadcastqVdqWqKmask
    CpuState::Evex, // EvexVbroadcastssVpsWss
    CpuState::Evex, // EvexVbroadcastssVpsWssKmask
    CpuState::Evex, // EvexVbroadcastsdVpdWsd
    CpuState::Evex, // EvexVbroadcastsdVpdWsdKmask
    CpuState::Evex, // EvexVmovqWqVq
    CpuState::Evex, // EvexVmovqVqWq
    CpuState::Evex, // EvexVinsertpsVpsWssIb
    CpuState::Evex, // EvexVextractpsEdVpsIb
    CpuState::Evex, // EvexVmovlpsVpsHpsMq
    CpuState::Evex, // EvexVmovhlpsVpsHpsWps
    CpuState::Evex, // EvexVmovhpsVpsHpsMq
    CpuState::Evex, // EvexVmovlhpsVpsHpsWps
    CpuState::Evex, // EvexVmovlpsMqVps
    CpuState::Evex, // EvexVmovhpsMqVps
    CpuState::Evex, // EvexVmovlpdMqVsd
    CpuState::Evex, // EvexVmovhpdMqVsd
    CpuState::Evex, // EvexVmovlpdVpdHpdMq
    CpuState::Evex, // EvexVmovhpdVpdHpdMq
    CpuState::Evex, // EvexVmovddupVpdWpd
    CpuState::Evex, // EvexVmovsldupVpsWps
    CpuState::Evex, // EvexVmovshdupVpsWps
    CpuState::Evex, // EvexVmovddupVpdWpdKmask
    CpuState::Evex, // EvexVmovsldupVpsWpsKmask
    CpuState::Evex, // EvexVmovshdupVpsWpsKmask
    CpuState::Evex, // EvexVpmovqbWdqVdq
    CpuState::Evex, // EvexVpmovdbWdqVdq
    CpuState::Evex, // EvexVpmovwbWdqVdq
    CpuState::Evex, // EvexVpmovdwWdqVdq
    CpuState::Evex, // EvexVpmovqwWdqVdq
    CpuState::Evex, // EvexVpmovqdWdqVdq
    CpuState::Evex, // EvexVpmovqbWdqVdqKmask
    CpuState::Evex, // EvexVpmovdbWdqVdqKmask
    CpuState::Evex, // EvexVpmovwbWdqVdqKmask
    CpuState::Evex, // EvexVpmovdwWdqVdqKmask
    CpuState::Evex, // EvexVpmovqwWdqVdqKmask
    CpuState::Evex, // EvexVpmovqdWdqVdqKmask
    CpuState::Evex, // EvexVpmovusqbWdqVdq
    CpuState::Evex, // EvexVpmovusdbWdqVdq
    CpuState::Evex, // EvexVpmovuswbWdqVdq
    CpuState::Evex, // EvexVpmovusdwWdqVdq
    CpuState::Evex, // EvexVpmovusqwWdqVdq
    CpuState::Evex, // EvexVpmovusqdWdqVdq
    CpuState::Evex, // EvexVpmovusqbWdqVdqKmask
    CpuState::Evex, // EvexVpmovusdbWdqVdqKmask
    CpuState::Evex, // EvexVpmovuswbWdqVdqKmask
    CpuState::Evex, // EvexVpmovusdwWdqVdqKmask
    CpuState::Evex, // EvexVpmovusqwWdqVdqKmask
    CpuState::Evex, // EvexVpmovusqdWdqVdqKmask
    CpuState::Evex, // EvexVpmovsqbWdqVdq
    CpuState::Evex, // EvexVpmovsdbWdqVdq
    CpuState::Evex, // EvexVpmovswbWdqVdq
    CpuState::Evex, // EvexVpmovsdwWdqVdq
    CpuState::Evex, // EvexVpmovsqwWdqVdq
    CpuState::Evex, // EvexVpmovsqdWdqVdq
    CpuState::Evex, // EvexVpmovsqbWdqVdqKmask
    CpuState::Evex, // EvexVpmovsdbWdqVdqKmask
    CpuState::Evex, // EvexVpmovswbWdqVdqKmask
    CpuState::Evex, // EvexVpmovsdwWdqVdqKmask
    CpuState::Evex, // EvexVpmovsqwWdqVdqKmask
    CpuState::Evex, // EvexVpmovsqdWdqVdqKmask
    CpuState::Evex, // EvexVpmovsxbwVdqWdq
    CpuState::Evex, // EvexVpmovsxbdVdqWdq
    CpuState::Evex, // EvexVpmovsxbqVdqWdq
    CpuState::Evex, // EvexVpmovsxwdVdqWdq
    CpuState::Evex, // EvexVpmovsxwqVdqWdq
    CpuState::Evex, // EvexVpmovsxdqVdqWdq
    CpuState::Evex, // EvexVpmovsxbwVdqWdqKmask
    CpuState::Evex, // EvexVpmovsxbdVdqWdqKmask
    CpuState::Evex, // EvexVpmovsxbqVdqWdqKmask
    CpuState::Evex, // EvexVpmovsxwdVdqWdqKmask
    CpuState::Evex, // EvexVpmovsxwqVdqWdqKmask
    CpuState::Evex, // EvexVpmovsxdqVdqWdqKmask
    CpuState::Evex, // EvexVpmovzxbwVdqWdq
    CpuState::Evex, // EvexVpmovzxbdVdqWdq
    CpuState::Evex, // EvexVpmovzxbqVdqWdq
    CpuState::Evex, // EvexVpmovzxwdVdqWdq
    CpuState::Evex, // EvexVpmovzxwqVdqWdq
    CpuState::Evex, // EvexVpmovzxdqVdqWdq
    CpuState::Evex, // EvexVpmovzxbwVdqWdqKmask
    CpuState::Evex, // EvexVpmovzxbdVdqWdqKmask
    CpuState::Evex, // EvexVpmovzxbqVdqWdqKmask
    CpuState::Evex, // EvexVpmovzxwdVdqWdqKmask
    CpuState::Evex, // EvexVpmovzxwqVdqWdqKmask
    CpuState::Evex, // EvexVpmovzxdqVdqWdqKmask
    CpuState::Evex, // EvexVpsubbVdqHdqWdq
    CpuState::Evex, // EvexVpsubsbVdqHdqWdq
    CpuState::Evex, // EvexVpsubusbVdqHdqWdq
    CpuState::Evex, // EvexVpsubwVdqHdqWdq
    CpuState::Evex, // EvexVpsubswVdqHdqWdq
    CpuState::Evex, // EvexVpsubuswVdqHdqWdq
    CpuState::Evex, // EvexVpaddbVdqHdqWdq
    CpuState::Evex, // EvexVpaddsbVdqHdqWdq
    CpuState::Evex, // EvexVpaddusbVdqHdqWdq
    CpuState::Evex, // EvexVpaddwVdqHdqWdq
    CpuState::Evex, // EvexVpaddswVdqHdqWdq
    CpuState::Evex, // EvexVpadduswVdqHdqWdq
    CpuState::Evex, // EvexVpsubbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubsbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubusbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubuswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpaddbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpaddsbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpaddusbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpaddwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpaddswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpadduswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminsbVdqHdqWdq
    CpuState::Evex, // EvexVpminubVdqHdqWdq
    CpuState::Evex, // EvexVpmaxubVdqHdqWdq
    CpuState::Evex, // EvexVpmaxsbVdqHdqWdq
    CpuState::Evex, // EvexVpminswVdqHdqWdq
    CpuState::Evex, // EvexVpminuwVdqHdqWdq
    CpuState::Evex, // EvexVpmaxswVdqHdqWdq
    CpuState::Evex, // EvexVpmaxuwVdqHdqWdq
    CpuState::Evex, // EvexVpminsbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminubVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxubVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxsbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminuwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxuwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpacksswbVdqHdqWdq
    CpuState::Evex, // EvexVpacksswbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpackuswbVdqHdqWdq
    CpuState::Evex, // EvexVpackuswbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpackssdwVdqHdqWdq
    CpuState::Evex, // EvexVpackssdwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpackusdwVdqHdqWdq
    CpuState::Evex, // EvexVpackusdwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpcklbwVdqHdqWdq
    CpuState::Evex, // EvexVpunpckhbwVdqHdqWdq
    CpuState::Evex, // EvexVpunpcklbwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpckhbwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpcklwdVdqHdqWdq
    CpuState::Evex, // EvexVpunpckhwdVdqHdqWdq
    CpuState::Evex, // EvexVpunpcklwdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpunpckhwdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpavgbVdqHdqWdq
    CpuState::Evex, // EvexVpavgwVdqHdqWdq
    CpuState::Evex, // EvexVpavgbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpavgwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaddubswVdqHdqWdq
    CpuState::Evex, // EvexVpmaddubswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmullwVdqHdqWdq
    CpuState::Evex, // EvexVpmulhwVdqHdqWdq
    CpuState::Evex, // EvexVpmulhuwVdqHdqWdq
    CpuState::Evex, // EvexVpmulhrswVdqHdqWdq
    CpuState::Evex, // EvexVpmullwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmulhwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmulhuwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmulhrswVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrldqUdqIb
    CpuState::Evex, // EvexVpslldqUdqIb
    CpuState::Evex, // EvexVpsadbwVdqHdqWdq
    CpuState::Evex, // EvexVpmaddwdVdqHdqWdq
    CpuState::Evex, // EvexVpmaddwdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmadd52luqVdqHdqWdq
    CpuState::Evex, // EvexVpmadd52luqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmadd52huqVdqHdqWdq
    CpuState::Evex, // EvexVpmadd52huqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmultishiftqbVdqHdqWdq
    CpuState::Evex, // EvexVpmultishiftqbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermbVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermt2bVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermt2wVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermi2bVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermi2wVdqHdqWdqKmask
    CpuState::Evex, // EvexVinsertf32x4VpsHpsWpsIb
    CpuState::Evex, // EvexVinsertf64x2VpdHpdWpdIb
    CpuState::Evex, // EvexVinsertf32x4VpsHpsWpsIbKmask
    CpuState::Evex, // EvexVinsertf64x2VpdHpdWpdIbKmask
    CpuState::Evex, // EvexVinsertf32x8VpsHpsWpsIb
    CpuState::Evex, // EvexVinsertf64x4VpdHpdWpdIb
    CpuState::Evex, // EvexVinsertf32x8VpsHpsWpsIbKmask
    CpuState::Evex, // EvexVinsertf64x4VpdHpdWpdIbKmask
    CpuState::Evex, // EvexVinserti32x4VdqHdqWdqIb
    CpuState::Evex, // EvexVinserti64x2VdqHdqWdqIb
    CpuState::Evex, // EvexVinserti32x4VdqHdqWdqIbKmask
    CpuState::Evex, // EvexVinserti64x2VdqHdqWdqIbKmask
    CpuState::Evex, // EvexVinserti32x8VdqHdqWdqIb
    CpuState::Evex, // EvexVinserti64x4VdqHdqWdqIb
    CpuState::Evex, // EvexVinserti32x8VdqHdqWdqIbKmask
    CpuState::Evex, // EvexVinserti64x4VdqHdqWdqIbKmask
    CpuState::Evex, // EvexVextractf32x4WpsVpsIb
    CpuState::Evex, // EvexVextractf64x2WpdVpdIb
    CpuState::Evex, // EvexVextractf32x4WpsVpsIbKmask
    CpuState::Evex, // EvexVextractf64x2WpdVpdIbKmask
    CpuState::Evex, // EvexVextractf32x8WpsVpsIb
    CpuState::Evex, // EvexVextractf64x4WpdVpdIb
    CpuState::Evex, // EvexVextractf32x8WpsVpsIbKmask
    CpuState::Evex, // EvexVextractf64x4WpdVpdIbKmask
    CpuState::Evex, // EvexVextracti32x4WdqVdqIb
    CpuState::Evex, // EvexVextracti64x2WdqVdqIb
    CpuState::Evex, // EvexVextracti32x4WdqVdqIbKmask
    CpuState::Evex, // EvexVextracti64x2WdqVdqIbKmask
    CpuState::Evex, // EvexVextracti32x8WdqVdqIb
    CpuState::Evex, // EvexVextracti64x4WdqVdqIb
    CpuState::Evex, // EvexVextracti32x8WdqVdqIbKmask
    CpuState::Evex, // EvexVextracti64x4WdqVdqIbKmask
    CpuState::Evex, // EvexVbroadcastf32x2VpsWq
    CpuState::Evex, // EvexVbroadcastf32x2VpsWqKmask
    CpuState::Evex, // EvexVbroadcasti32x2VdqWq
    CpuState::Evex, // EvexVbroadcasti32x2VdqWqKmask
    CpuState::Evex, // EvexVbroadcastf32x4VpsWps
    CpuState::Evex, // EvexVbroadcastf64x2VpdWpd
    CpuState::Evex, // EvexVbroadcastf32x4VpsWpsKmask
    CpuState::Evex, // EvexVbroadcastf64x2VpdWpdKmask
    CpuState::Evex, // EvexVbroadcastf32x8VpsWps
    CpuState::Evex, // EvexVbroadcastf64x4VpdWpd
    CpuState::Evex, // EvexVbroadcastf32x8VpsWpsKmask
    CpuState::Evex, // EvexVbroadcastf64x4VpdWpdKmask
    CpuState::Evex, // EvexVbroadcasti32x4VdqWdq
    CpuState::Evex, // EvexVbroadcasti64x2VdqWdq
    CpuState::Evex, // EvexVbroadcasti32x4VdqWdqKmask
    CpuState::Evex, // EvexVbroadcasti64x2VdqWdqKmask
    CpuState::Evex, // EvexVbroadcasti32x8VdqWdq
    CpuState::Evex, // EvexVbroadcasti64x4VdqWdq
    CpuState::Evex, // EvexVbroadcasti32x8VdqWdqKmask
    CpuState::Evex, // EvexVbroadcasti64x4VdqWdqKmask
    CpuState::Evex, // EvexVpmulldVdqHdqWdq
    CpuState::Evex, // EvexVpmullqVdqHdqWdq
    CpuState::Evex, // EvexVpmulldVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmullqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpadddVdqHdqWdq
    CpuState::Evex, // EvexVpaddqVdqHdqWdq
    CpuState::Evex, // EvexVpadddVdqHdqWdqKmask
    CpuState::Evex, // EvexVpaddqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubdVdqHdqWdq
    CpuState::Evex, // EvexVpsubqVdqHdqWdq
    CpuState::Evex, // EvexVpsubdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsubqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpanddVdqHdqWdq
    CpuState::Evex, // EvexVpandqVdqHdqWdq
    CpuState::Evex, // EvexVpanddVdqHdqWdqKmask
    CpuState::Evex, // EvexVpandqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpandndVdqHdqWdq
    CpuState::Evex, // EvexVpandnqVdqHdqWdq
    CpuState::Evex, // EvexVpandndVdqHdqWdqKmask
    CpuState::Evex, // EvexVpandnqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpordVdqHdqWdq
    CpuState::Evex, // EvexVporqVdqHdqWdq
    CpuState::Evex, // EvexVpordVdqHdqWdqKmask
    CpuState::Evex, // EvexVporqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpxordVdqHdqWdq
    CpuState::Evex, // EvexVpxorqVdqHdqWdq
    CpuState::Evex, // EvexVpxordVdqHdqWdqKmask
    CpuState::Evex, // EvexVpxorqVdqHdqWdqKmask
    CpuState::Evex, // EvexVandpsVpsHpsWps
    CpuState::Evex, // EvexVandpdVpdHpdWpd
    CpuState::Evex, // EvexVandpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVandpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVandnpsVpsHpsWps
    CpuState::Evex, // EvexVandnpdVpdHpdWpd
    CpuState::Evex, // EvexVandnpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVandnpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVorpsVpsHpsWps
    CpuState::Evex, // EvexVorpdVpdHpdWpd
    CpuState::Evex, // EvexVorpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVorpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVxorpsVpsHpsWps
    CpuState::Evex, // EvexVxorpdVpdHpdWpd
    CpuState::Evex, // EvexVxorpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVxorpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVpmaxsdVdqHdqWdq
    CpuState::Evex, // EvexVpmaxsqVdqHdqWdq
    CpuState::Evex, // EvexVpmaxsdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxsqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxudVdqHdqWdq
    CpuState::Evex, // EvexVpmaxuqVdqHdqWdq
    CpuState::Evex, // EvexVpmaxudVdqHdqWdqKmask
    CpuState::Evex, // EvexVpmaxuqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminsdVdqHdqWdq
    CpuState::Evex, // EvexVpminsqVdqHdqWdq
    CpuState::Evex, // EvexVpminsdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminsqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminudVdqHdqWdq
    CpuState::Evex, // EvexVpminuqVdqHdqWdq
    CpuState::Evex, // EvexVpminudVdqHdqWdqKmask
    CpuState::Evex, // EvexVpminuqVdqHdqWdqKmask
    CpuState::Evex, // EvexValigndVdqHdqWdqIbKmask
    CpuState::Evex, // EvexValignqVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpalignrVdqHdqWdqIb
    CpuState::Evex, // EvexVpalignrVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVdbpsadbwVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpsrlvwVdqHdqWdq
    CpuState::Evex, // EvexVpsrlvdVdqHdqWdq
    CpuState::Evex, // EvexVpsrlvqVdqHdqWdq
    CpuState::Evex, // EvexVpsravwVdqHdqWdq
    CpuState::Evex, // EvexVpsravdVdqHdqWdq
    CpuState::Evex, // EvexVpsravqVdqHdqWdq
    CpuState::Evex, // EvexVpsllvwVdqHdqWdq
    CpuState::Evex, // EvexVpsllvdVdqHdqWdq
    CpuState::Evex, // EvexVpsllvqVdqHdqWdq
    CpuState::Evex, // EvexVprolvdVdqHdqWdq
    CpuState::Evex, // EvexVprolvqVdqHdqWdq
    CpuState::Evex, // EvexVprorvdVdqHdqWdq
    CpuState::Evex, // EvexVprorvqVdqHdqWdq
    CpuState::Evex, // EvexVpsrlvwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrlvdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsrlvqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsravwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsravdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsravqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsllvwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsllvdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsllvqVdqHdqWdqKmask
    CpuState::Evex, // EvexVprolvdVdqHdqWdqKmask
    CpuState::Evex, // EvexVprolvqVdqHdqWdqKmask
    CpuState::Evex, // EvexVprorvdVdqHdqWdqKmask
    CpuState::Evex, // EvexVprorvqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsradVdqHdqWdq
    CpuState::Evex, // EvexVpsraqVdqHdqWdq
    CpuState::Evex, // EvexVpsradVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsraqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpsradUdqIb
    CpuState::Evex, // EvexVpsraqUdqIb
    CpuState::Evex, // EvexVprordUdqIb
    CpuState::Evex, // EvexVprorqUdqIb
    CpuState::Evex, // EvexVproldUdqIb
    CpuState::Evex, // EvexVprolqUdqIb
    CpuState::Evex, // EvexVpsradUdqIbKmask
    CpuState::Evex, // EvexVpsraqUdqIbKmask
    CpuState::Evex, // EvexVprordUdqIbKmask
    CpuState::Evex, // EvexVprorqUdqIbKmask
    CpuState::Evex, // EvexVproldUdqIbKmask
    CpuState::Evex, // EvexVprolqUdqIbKmask
    CpuState::Evex, // EvexVmovdqu8VdqWdq
    CpuState::Evex, // EvexVmovdqu16VdqWdq
    CpuState::Evex, // EvexVmovdqu8VdqWdqKmask
    CpuState::Evex, // EvexVmovdqu16VdqWdqKmask
    CpuState::Evex, // EvexVmovdqu8WdqVdq
    CpuState::Evex, // EvexVmovdqu16WdqVdq
    CpuState::Evex, // EvexVmovdqu8WdqVdqKmask
    CpuState::Evex, // EvexVmovdqu16WdqVdqKmask
    CpuState::Evex, // EvexVmovdqu32VdqWdq
    CpuState::Evex, // EvexVmovdqu64VdqWdq
    CpuState::Evex, // EvexVmovdqu32VdqWdqKmask
    CpuState::Evex, // EvexVmovdqu64VdqWdqKmask
    CpuState::Evex, // EvexVmovdqu32WdqVdq
    CpuState::Evex, // EvexVmovdqu64WdqVdq
    CpuState::Evex, // EvexVmovdqu32WdqVdqKmask
    CpuState::Evex, // EvexVmovdqu64WdqVdqKmask
    CpuState::Evex, // EvexVmovdqa32VdqWdq
    CpuState::Evex, // EvexVmovdqa64VdqWdq
    CpuState::Evex, // EvexVmovdqa32VdqWdqKmask
    CpuState::Evex, // EvexVmovdqa64VdqWdqKmask
    CpuState::Evex, // EvexVmovdqa32WdqVdq
    CpuState::Evex, // EvexVmovdqa64WdqVdq
    CpuState::Evex, // EvexVmovdqa32WdqVdqKmask
    CpuState::Evex, // EvexVmovdqa64WdqVdqKmask
    CpuState::Evex, // EvexVrangepsVpsHpsWpsIbKmask
    CpuState::Evex, // EvexVrangepdVpdHpdWpdIbKmask
    CpuState::Evex, // EvexVrangessVssHpsWssIbKmask
    CpuState::Evex, // EvexVrangesdVsdHpdWsdIbKmask
    CpuState::Evex, // EvexVgetexppsVpsWps
    CpuState::Evex, // EvexVgetexppdVpdWpd
    CpuState::Evex, // EvexVgetexpssVssHpsWss
    CpuState::Evex, // EvexVgetexpsdVsdHpdWsd
    CpuState::Evex, // EvexVgetexppsVpsWpsKmask
    CpuState::Evex, // EvexVgetexppdVpdWpdKmask
    CpuState::Evex, // EvexVgetexpssVssHpsWssKmask
    CpuState::Evex, // EvexVgetexpsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVgetmantpsVpsWpsIbKmask
    CpuState::Evex, // EvexVgetmantpdVpdWpdIbKmask
    CpuState::Evex, // EvexVgetmantssVssHpsWssIbKmask
    CpuState::Evex, // EvexVgetmantsdVsdHpdWsdIbKmask
    CpuState::Evex, // EvexVscalefpsVpsHpsWps
    CpuState::Evex, // EvexVscalefpdVpdHpdWpd
    CpuState::Evex, // EvexVscalefssVssHpsWss
    CpuState::Evex, // EvexVscalefsdVsdHpdWsd
    CpuState::Evex, // EvexVscalefpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVscalefpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVscalefssVssHpsWssKmask
    CpuState::Evex, // EvexVscalefsdVsdHpdWsdKmask
    CpuState::Evex, // EvexVrcp14psVpsWpsKmask
    CpuState::Evex, // EvexVrcp14pdVpdWpdKmask
    CpuState::Evex, // EvexVrcp14ssVssHpsWssKmask
    CpuState::Evex, // EvexVrcp14sdVsdHpdWsdKmask
    CpuState::Evex, // EvexVrsqrt14psVpsWpsKmask
    CpuState::Evex, // EvexVrsqrt14pdVpdWpdKmask
    CpuState::Evex, // EvexVrsqrt14ssVssHpsWssKmask
    CpuState::Evex, // EvexVrsqrt14sdVsdHpdWsdKmask
    CpuState::Evex, // EvexVcvtps2uqqVdqWps
    CpuState::Evex, // EvexVcvtpd2uqqVdqWpd
    CpuState::Evex, // EvexVcvtps2uqqVdqWpsKmask
    CpuState::Evex, // EvexVcvtpd2uqqVdqWpdKmask
    CpuState::Evex, // EvexVcvttps2uqqVdqWps
    CpuState::Evex, // EvexVcvttps2uqqVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2uqqVdqWpd
    CpuState::Evex, // EvexVcvttpd2uqqVdqWpdKmask
    CpuState::Evex, // EvexVcvtps2qqVdqWps
    CpuState::Evex, // EvexVcvtps2qqVdqWpsKmask
    CpuState::Evex, // EvexVcvtpd2qqVdqWpd
    CpuState::Evex, // EvexVcvtpd2qqVdqWpdKmask
    CpuState::Evex, // EvexVcvttps2qqVdqWps
    CpuState::Evex, // EvexVcvttps2qqVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2qqVdqWpd
    CpuState::Evex, // EvexVcvttpd2qqVdqWpdKmask
    CpuState::Evex, // EvexVcvttps2udqVdqWps
    CpuState::Evex, // EvexVcvttpd2udqVdqWpd
    CpuState::Evex, // EvexVcvttps2udqVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2udqVdqWpdKmask
    CpuState::Evex, // EvexVcvtps2udqVdqWps
    CpuState::Evex, // EvexVcvtpd2udqVdqWpd
    CpuState::Evex, // EvexVcvtps2udqVdqWpsKmask
    CpuState::Evex, // EvexVcvtpd2udqVdqWpdKmask
    CpuState::Evex, // EvexVcvtudq2pdVpdWdq
    CpuState::Evex, // EvexVcvtudq2pdVpdWdqKmask
    CpuState::Evex, // EvexVcvtuqq2pdVpdWdq
    CpuState::Evex, // EvexVcvtuqq2pdVpdWdqKmask
    CpuState::Evex, // EvexVcvtudq2psVpsWdq
    CpuState::Evex, // EvexVcvtudq2psVpsWdqKmask
    CpuState::Evex, // EvexVcvtuqq2psVpsWdq
    CpuState::Evex, // EvexVcvtuqq2psVpsWdqKmask
    CpuState::Evex, // EvexVcvtdq2pdVpdWdq
    CpuState::Evex, // EvexVcvtdq2pdVpdWdqKmask
    CpuState::Evex, // EvexVcvtqq2pdVpdWdq
    CpuState::Evex, // EvexVcvtqq2pdVpdWdqKmask
    CpuState::Evex, // EvexVcvtdq2psVpsWdq
    CpuState::Evex, // EvexVcvtdq2psVpsWdqKmask
    CpuState::Evex, // EvexVcvtqq2psVpsWdq
    CpuState::Evex, // EvexVcvtqq2psVpsWdqKmask
    CpuState::Evex, // EvexVfmadd132psVpsHpsWps
    CpuState::Evex, // EvexVfmadd132pdVpdHpdWpd
    CpuState::Evex, // EvexVfmadd213psVpsHpsWps
    CpuState::Evex, // EvexVfmadd213pdVpdHpdWpd
    CpuState::Evex, // EvexVfmadd231psVpsHpsWps
    CpuState::Evex, // EvexVfmadd231pdVpdHpdWpd
    CpuState::Evex, // EvexVfmadd132psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmadd132pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmadd213psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmadd213pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmadd231psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmadd231pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmadd132ssVpsHssWss
    CpuState::Evex, // EvexVfmadd132sdVpdHsdWsd
    CpuState::Evex, // EvexVfmadd213ssVpsHssWss
    CpuState::Evex, // EvexVfmadd213sdVpdHsdWsd
    CpuState::Evex, // EvexVfmadd231ssVpsHssWss
    CpuState::Evex, // EvexVfmadd231sdVpdHsdWsd
    CpuState::Evex, // EvexVfmadd132ssVpsHssWssKmask
    CpuState::Evex, // EvexVfmadd132sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfmadd213ssVpsHssWssKmask
    CpuState::Evex, // EvexVfmadd213sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfmadd231ssVpsHssWssKmask
    CpuState::Evex, // EvexVfmadd231sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfmaddsub132psVpsHpsWps
    CpuState::Evex, // EvexVfmaddsub132pdVpdHpdWpd
    CpuState::Evex, // EvexVfmaddsub213psVpsHpsWps
    CpuState::Evex, // EvexVfmaddsub213pdVpdHpdWpd
    CpuState::Evex, // EvexVfmaddsub231psVpsHpsWps
    CpuState::Evex, // EvexVfmaddsub231pdVpdHpdWpd
    CpuState::Evex, // EvexVfmaddsub132psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmaddsub132pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmaddsub213psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmaddsub213pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmaddsub231psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmaddsub231pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsubadd132psVpsHpsWps
    CpuState::Evex, // EvexVfmsubadd132pdVpdHpdWpd
    CpuState::Evex, // EvexVfmsubadd213psVpsHpsWps
    CpuState::Evex, // EvexVfmsubadd213pdVpdHpdWpd
    CpuState::Evex, // EvexVfmsubadd231psVpsHpsWps
    CpuState::Evex, // EvexVfmsubadd231pdVpdHpdWpd
    CpuState::Evex, // EvexVfmsubadd132psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmsubadd132pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsubadd213psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmsubadd213pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsubadd231psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmsubadd231pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsub132psVpsHpsWps
    CpuState::Evex, // EvexVfmsub132pdVpdHpdWpd
    CpuState::Evex, // EvexVfmsub213psVpsHpsWps
    CpuState::Evex, // EvexVfmsub213pdVpdHpdWpd
    CpuState::Evex, // EvexVfmsub231psVpsHpsWps
    CpuState::Evex, // EvexVfmsub231pdVpdHpdWpd
    CpuState::Evex, // EvexVfmsub132psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmsub132pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsub213psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmsub213pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsub231psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfmsub231pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfmsub132ssVpsHssWss
    CpuState::Evex, // EvexVfmsub132sdVpdHsdWsd
    CpuState::Evex, // EvexVfmsub213ssVpsHssWss
    CpuState::Evex, // EvexVfmsub213sdVpdHsdWsd
    CpuState::Evex, // EvexVfmsub231ssVpsHssWss
    CpuState::Evex, // EvexVfmsub231sdVpdHsdWsd
    CpuState::Evex, // EvexVfmsub132ssVpsHssWssKmask
    CpuState::Evex, // EvexVfmsub132sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfmsub213ssVpsHssWssKmask
    CpuState::Evex, // EvexVfmsub213sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfmsub231ssVpsHssWssKmask
    CpuState::Evex, // EvexVfmsub231sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfnmadd132psVpsHpsWps
    CpuState::Evex, // EvexVfnmadd132pdVpdHpdWpd
    CpuState::Evex, // EvexVfnmadd213psVpsHpsWps
    CpuState::Evex, // EvexVfnmadd213pdVpdHpdWpd
    CpuState::Evex, // EvexVfnmadd231psVpsHpsWps
    CpuState::Evex, // EvexVfnmadd231pdVpdHpdWpd
    CpuState::Evex, // EvexVfnmadd132psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfnmadd132pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfnmadd213psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfnmadd213pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfnmadd231psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfnmadd231pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfnmadd132ssVpsHssWss
    CpuState::Evex, // EvexVfnmadd132sdVpdHsdWsd
    CpuState::Evex, // EvexVfnmadd213ssVpsHssWss
    CpuState::Evex, // EvexVfnmadd213sdVpdHsdWsd
    CpuState::Evex, // EvexVfnmadd231ssVpsHssWss
    CpuState::Evex, // EvexVfnmadd231sdVpdHsdWsd
    CpuState::Evex, // EvexVfnmadd132ssVpsHssWssKmask
    CpuState::Evex, // EvexVfnmadd132sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfnmadd213ssVpsHssWssKmask
    CpuState::Evex, // EvexVfnmadd213sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfnmadd231ssVpsHssWssKmask
    CpuState::Evex, // EvexVfnmadd231sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfnmsub132psVpsHpsWps
    CpuState::Evex, // EvexVfnmsub132pdVpdHpdWpd
    CpuState::Evex, // EvexVfnmsub213psVpsHpsWps
    CpuState::Evex, // EvexVfnmsub213pdVpdHpdWpd
    CpuState::Evex, // EvexVfnmsub231psVpsHpsWps
    CpuState::Evex, // EvexVfnmsub231pdVpdHpdWpd
    CpuState::Evex, // EvexVfnmsub132psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfnmsub132pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfnmsub213psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfnmsub213pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfnmsub231psVpsHpsWpsKmask
    CpuState::Evex, // EvexVfnmsub231pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVfnmsub132ssVpsHssWss
    CpuState::Evex, // EvexVfnmsub132sdVpdHsdWsd
    CpuState::Evex, // EvexVfnmsub213ssVpsHssWss
    CpuState::Evex, // EvexVfnmsub213sdVpdHsdWsd
    CpuState::Evex, // EvexVfnmsub231ssVpsHssWss
    CpuState::Evex, // EvexVfnmsub231sdVpdHsdWsd
    CpuState::Evex, // EvexVfnmsub132ssVpsHssWssKmask
    CpuState::Evex, // EvexVfnmsub132sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfnmsub213ssVpsHssWssKmask
    CpuState::Evex, // EvexVfnmsub213sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVfnmsub231ssVpsHssWssKmask
    CpuState::Evex, // EvexVfnmsub231sdVpdHsdWsdKmask
    CpuState::Evex, // EvexVpcmpbKgqHdqWdqIb
    CpuState::Evex, // EvexVpcmpwKgdHdqWdqIb
    CpuState::Evex, // EvexVpcmpubKgqHdqWdqIb
    CpuState::Evex, // EvexVpcmpuwKgdHdqWdqIb
    CpuState::Evex, // EvexVpcmpdKgwHdqWdqIb
    CpuState::Evex, // EvexVpcmpqKgbHdqWdqIb
    CpuState::Evex, // EvexVpcmpudKgwHdqWdqIb
    CpuState::Evex, // EvexVpcmpuqKgbHdqWdqIb
    CpuState::Evex, // EvexVptestmbKgqHdqWdq
    CpuState::Evex, // EvexVptestmwKgdHdqWdq
    CpuState::Evex, // EvexVptestnmbKgqHdqWdq
    CpuState::Evex, // EvexVptestnmwKgdHdqWdq
    CpuState::Evex, // EvexVptestmdKgwHdqWdq
    CpuState::Evex, // EvexVptestmqKgbHdqWdq
    CpuState::Evex, // EvexVptestnmdKgwHdqWdq
    CpuState::Evex, // EvexVptestnmqKgbHdqWdq
    CpuState::Evex, // EvexVpternlogdVdqHdqWdqIb
    CpuState::Evex, // EvexVpternlogqVdqHdqWdqIb
    CpuState::Evex, // EvexVpternlogdVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpternlogqVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVgatherdpsVpsVsib
    CpuState::Evex, // EvexVgatherdpdVpdVsib
    CpuState::Evex, // EvexVgatherqpsVpsVsib
    CpuState::Evex, // EvexVgatherqpdVpdVsib
    CpuState::Evex, // EvexVgatherddVdqVsib
    CpuState::Evex, // EvexVgatherdqVdqVsib
    CpuState::Evex, // EvexVgatherqdVdqVsib
    CpuState::Evex, // EvexVgatherqqVdqVsib
    CpuState::Evex, // EvexVscatterdpsVsibVps
    CpuState::Evex, // EvexVscatterdpdVsibVpd
    CpuState::Evex, // EvexVscatterqpsVsibVps
    CpuState::Evex, // EvexVscatterqpdVsibVpd
    CpuState::Evex, // EvexVscatterddVsibVdq
    CpuState::Evex, // EvexVscatterdqVsibVdq
    CpuState::Evex, // EvexVscatterqdVsibVdq
    CpuState::Evex, // EvexVscatterqqVsibVdq
    CpuState::Evex, // EvexVblendmpsVpsHpsWps
    CpuState::Evex, // EvexVblendmpdVpdHpdWpd
    CpuState::Evex, // EvexVpblendmdVdqHdqWdq
    CpuState::Evex, // EvexVpblendmqVdqHdqWdq
    CpuState::Evex, // EvexVpblendmbVdqHdqWdq
    CpuState::Evex, // EvexVpblendmwVdqHdqWdq
    CpuState::Evex, // EvexVshufi32x4VdqHdqWdqIbKmask
    CpuState::Evex, // EvexVshufi64x2VdqHdqWdqIbKmask
    CpuState::Evex, // EvexVshuff32x4VpsHpsWpsIbKmask
    CpuState::Evex, // EvexVshuff64x2VpdHpdWpdIbKmask
    CpuState::Evex, // EvexVexpandpsVpsWps
    CpuState::Evex, // EvexVexpandpdVpdWpd
    CpuState::Evex, // EvexVexpandpsVpsWpsKmask
    CpuState::Evex, // EvexVexpandpdVpdWpdKmask
    CpuState::Evex, // EvexVcompresspsWpsVps
    CpuState::Evex, // EvexVcompresspdWpdVpd
    CpuState::Evex, // EvexVcompresspsWpsVpsKmask
    CpuState::Evex, // EvexVcompresspdWpdVpdKmask
    CpuState::Evex, // EvexVpexpandbVdqWdq
    CpuState::Evex, // EvexVpexpandwVdqWdq
    CpuState::Evex, // EvexVpexpandbVdqWdqKmask
    CpuState::Evex, // EvexVpexpandwVdqWdqKmask
    CpuState::Evex, // EvexVpexpanddVdqWdq
    CpuState::Evex, // EvexVpexpandqVdqWdq
    CpuState::Evex, // EvexVpexpanddVdqWdqKmask
    CpuState::Evex, // EvexVpexpandqVdqWdqKmask
    CpuState::Evex, // EvexVpcompressbWdqVdq
    CpuState::Evex, // EvexVpcompresswWdqVdq
    CpuState::Evex, // EvexVpcompressbWdqVdqKmask
    CpuState::Evex, // EvexVpcompresswWdqVdqKmask
    CpuState::Evex, // EvexVpcompressdWdqVdq
    CpuState::Evex, // EvexVpcompressqWdqVdq
    CpuState::Evex, // EvexVpcompressdWdqVdqKmask
    CpuState::Evex, // EvexVpcompressqWdqVdqKmask
    CpuState::Evex, // EvexVfixupimmssVssHssWssIbKmask
    CpuState::Evex, // EvexVfixupimmsdVsdHsdWsdIbKmask
    CpuState::Evex, // EvexVfixupimmpsVpsHpsWpsIb
    CpuState::Evex, // EvexVfixupimmpdVpdHpdWpdIb
    CpuState::Evex, // EvexVfixupimmpsVpsHpsWpsIbKmask
    CpuState::Evex, // EvexVfixupimmpdVpdHpdWpdIbKmask
    CpuState::Evex, // EvexVfpclasspsKgwWpsIbKmask
    CpuState::Evex, // EvexVfpclasspdKgbWpdIbKmask
    CpuState::Evex, // EvexVfpclassssKgbWssIbKmask
    CpuState::Evex, // EvexVfpclasssdKgbWsdIbKmask
    CpuState::Evex, // EvexVreducepsVpsWpsIbKmask
    CpuState::Evex, // EvexVreducepdVpdWpdIbKmask
    CpuState::Evex, // EvexVreducessVssHpsWssIbKmask
    CpuState::Evex, // EvexVreducesdVsdHpdWsdIbKmask
    CpuState::Evex, // EvexVpermt2dVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermt2qVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermi2dVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermi2qVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermt2psVpsHpsWpsKmask
    CpuState::Evex, // EvexVpermt2pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVpermi2psVpsHpsWpsKmask
    CpuState::Evex, // EvexVpermi2pdVpdHpdWpdKmask
    CpuState::Evex, // EvexVpermdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpermpsVpsHpsWpsKmask
    CpuState::Evex, // EvexVpermpdVpdHpdWpdKmask
    CpuState::Evex, // EvexVpconflictdVdqWdqKmask
    CpuState::Evex, // EvexVpconflictqVdqWdqKmask
    CpuState::Evex, // EvexVplzcntdVdqWdqKmask
    CpuState::Evex, // EvexVplzcntqVdqWdqKmask
    CpuState::Evex, // EvexVpmovm2bVdqKeq
    CpuState::Evex, // EvexVpmovm2wVdqKed
    CpuState::Evex, // EvexVpmovm2dVdqKew
    CpuState::Evex, // EvexVpmovm2qVdqKeb
    CpuState::Evex, // EvexVpmovb2mKgqWdq
    CpuState::Evex, // EvexVpmovw2mKgdWdq
    CpuState::Evex, // EvexVpmovd2mKgwWdq
    CpuState::Evex, // EvexVpmovq2mKgbWdq
    CpuState::Evex, // EvexVpopcntbVdqWdqKmask
    CpuState::Evex, // EvexVpopcntwVdqWdqKmask
    CpuState::Evex, // EvexVpopcntdVdqWdqKmask
    CpuState::Evex, // EvexVpopcntqVdqWdqKmask
    CpuState::Evex, // EvexVpshrddVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpshrdqVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpshrdvdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpshrdvqVdqHdqWdqKmask
    CpuState::Evex, // EvexVpshlddVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpshldqVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpshldvdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpshldvqVdqHdqWdqKmask
    CpuState::Evex, // EvexVcvtss2siGdWss
    CpuState::Evex, // EvexVcvtss2siGqWss
    CpuState::Evex, // EvexVcvtsd2siGdWsd
    CpuState::Evex, // EvexVcvtsd2siGqWsd
    CpuState::Evex, // EvexVcvttss2siGdWss
    CpuState::Evex, // EvexVcvttss2siGqWss
    CpuState::Evex, // EvexVcvttsd2siGdWsd
    CpuState::Evex, // EvexVcvttsd2siGqWsd
    CpuState::Evex, // EvexVmovdVdqEd
    CpuState::Evex, // EvexVmovqVdqEq
    CpuState::Evex, // EvexVmovdEdVd
    CpuState::Evex, // EvexVmovqEqVq
    CpuState::Evex, // EvexVcvtsi2ssVssEd
    CpuState::Evex, // EvexVcvtsi2ssVssEq
    CpuState::Evex, // EvexVcvtsi2sdVsdEd
    CpuState::Evex, // EvexVcvtsi2sdVsdEq
    CpuState::Evex, // EvexVcvtusi2ssVssEd
    CpuState::Evex, // EvexVcvtusi2ssVssEq
    CpuState::Evex, // EvexVcvtusi2sdVsdEd
    CpuState::Evex, // EvexVcvtusi2sdVsdEq
    CpuState::Evex, // EvexVcvtss2usiGdWss
    CpuState::Evex, // EvexVcvtss2usiGqWss
    CpuState::Evex, // EvexVcvtsd2usiGdWsd
    CpuState::Evex, // EvexVcvtsd2usiGqWsd
    CpuState::Evex, // EvexVcvttss2usiGdWss
    CpuState::Evex, // EvexVcvttss2usiGqWss
    CpuState::Evex, // EvexVcvttsd2usiGdWsd
    CpuState::Evex, // EvexVcvttsd2usiGqWsd
    CpuState::Evex, // EvexVpinsrbVdqEbIb
    CpuState::Evex, // EvexVpinsrwVdqEwIb
    CpuState::Evex, // EvexVpextrwGdUdqIb
    CpuState::Evex, // EvexVpextrbEdVdqIbR
    CpuState::Evex, // EvexVpextrbMbVdqIbM
    CpuState::Evex, // EvexVpextrwEdVdqIbR
    CpuState::Evex, // EvexVpextrwMwVdqIbM
    CpuState::Evex, // EvexVpinsrdVdqEdIb
    CpuState::Evex, // EvexVpinsrqVdqEqIb
    CpuState::Evex, // EvexVpextrdEdVdqIb
    CpuState::Evex, // EvexVpextrqEqVdqIb
    CpuState::Evex, // EvexVpbroadcastmb2qVdqKeb
    CpuState::Evex, // EvexVpbroadcastmw2dVdqKew
    CpuState::Evex, // EvexVpdpbusdVdqHdqWdq
    CpuState::Evex, // EvexVpdpbusdsVdqHdqWdq
    CpuState::Evex, // EvexVpdpwssdVdqHdqWdq
    CpuState::Evex, // EvexVpdpwssdsVdqHdqWdq
    CpuState::Evex, // EvexVpdpbusdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpbusdsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwssdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwssdsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpshufbitqmbKgqHdqWdqKmask
    CpuState::Evex, // EvexVp2intersectdKgqHdqWdq
    CpuState::Evex, // EvexVp2intersectqKgqHdqWdq
    CpuState::Evex, // EvexVpshrdwVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpshrdvwVdqHdqWdqKmask
    CpuState::Evex, // EvexVpshldwVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVpshldvwVdqHdqWdqKmask
    CpuState::Evex, // EvexVaddshVshHphWsh
    CpuState::Evex, // EvexVaddshVshHphWshKmask
    CpuState::Evex, // EvexVsubshVshHphWsh
    CpuState::Evex, // EvexVsubshVshHphWshKmask
    CpuState::Evex, // EvexVmulshVshHphWsh
    CpuState::Evex, // EvexVmulshVshHphWshKmask
    CpuState::Evex, // EvexVdivshVshHphWsh
    CpuState::Evex, // EvexVdivshVshHphWshKmask
    CpuState::Evex, // EvexVminshVshHphWsh
    CpuState::Evex, // EvexVminshVshHphWshKmask
    CpuState::Evex, // EvexVmaxshVshHphWsh
    CpuState::Evex, // EvexVmaxshVshHphWshKmask
    CpuState::Evex, // EvexVscalefshVshHphWsh
    CpuState::Evex, // EvexVscalefshVshHphWshKmask
    CpuState::Evex, // EvexVaddphVphHphWph
    CpuState::Evex, // EvexVaddphVphHphWphKmask
    CpuState::Evex, // EvexVsubphVphHphWph
    CpuState::Evex, // EvexVsubphVphHphWphKmask
    CpuState::Evex, // EvexVmulphVphHphWph
    CpuState::Evex, // EvexVmulphVphHphWphKmask
    CpuState::Evex, // EvexVdivphVphHphWph
    CpuState::Evex, // EvexVdivphVphHphWphKmask
    CpuState::Evex, // EvexVminphVphHphWph
    CpuState::Evex, // EvexVminphVphHphWphKmask
    CpuState::Evex, // EvexVmaxphVphHphWph
    CpuState::Evex, // EvexVmaxphVphHphWphKmask
    CpuState::Evex, // EvexVscalefphVphHphWph
    CpuState::Evex, // EvexVscalefphVphHphWphKmask
    CpuState::Evex, // EvexVfmadd132shVphHshWsh
    CpuState::Evex, // EvexVfmadd132shVphHshWshKmask
    CpuState::Evex, // EvexVfmadd213shVphHshWsh
    CpuState::Evex, // EvexVfmadd213shVphHshWshKmask
    CpuState::Evex, // EvexVfmadd231shVphHshWsh
    CpuState::Evex, // EvexVfmadd231shVphHshWshKmask
    CpuState::Evex, // EvexVfnmadd132shVphHshWsh
    CpuState::Evex, // EvexVfnmadd132shVphHshWshKmask
    CpuState::Evex, // EvexVfnmadd213shVphHshWsh
    CpuState::Evex, // EvexVfnmadd213shVphHshWshKmask
    CpuState::Evex, // EvexVfnmadd231shVphHshWsh
    CpuState::Evex, // EvexVfnmadd231shVphHshWshKmask
    CpuState::Evex, // EvexVfmsub132shVphHshWsh
    CpuState::Evex, // EvexVfmsub132shVphHshWshKmask
    CpuState::Evex, // EvexVfmsub213shVphHshWsh
    CpuState::Evex, // EvexVfmsub213shVphHshWshKmask
    CpuState::Evex, // EvexVfmsub231shVphHshWsh
    CpuState::Evex, // EvexVfmsub231shVphHshWshKmask
    CpuState::Evex, // EvexVfnmsub132shVphHshWsh
    CpuState::Evex, // EvexVfnmsub132shVphHshWshKmask
    CpuState::Evex, // EvexVfnmsub213shVphHshWsh
    CpuState::Evex, // EvexVfnmsub213shVphHshWshKmask
    CpuState::Evex, // EvexVfnmsub231shVphHshWsh
    CpuState::Evex, // EvexVfnmsub231shVphHshWshKmask
    CpuState::Evex, // EvexVfmadd132phVphHphWph
    CpuState::Evex, // EvexVfmadd132phVphHphWphKmask
    CpuState::Evex, // EvexVfmadd213phVphHphWph
    CpuState::Evex, // EvexVfmadd213phVphHphWphKmask
    CpuState::Evex, // EvexVfmadd231phVphHphWph
    CpuState::Evex, // EvexVfmadd231phVphHphWphKmask
    CpuState::Evex, // EvexVfnmadd132phVphHphWph
    CpuState::Evex, // EvexVfnmadd132phVphHphWphKmask
    CpuState::Evex, // EvexVfnmadd213phVphHphWph
    CpuState::Evex, // EvexVfnmadd213phVphHphWphKmask
    CpuState::Evex, // EvexVfnmadd231phVphHphWph
    CpuState::Evex, // EvexVfnmadd231phVphHphWphKmask
    CpuState::Evex, // EvexVfmsub132phVphHphWph
    CpuState::Evex, // EvexVfmsub132phVphHphWphKmask
    CpuState::Evex, // EvexVfmsub213phVphHphWph
    CpuState::Evex, // EvexVfmsub213phVphHphWphKmask
    CpuState::Evex, // EvexVfmsub231phVphHphWph
    CpuState::Evex, // EvexVfmsub231phVphHphWphKmask
    CpuState::Evex, // EvexVfnmsub132phVphHphWph
    CpuState::Evex, // EvexVfnmsub132phVphHphWphKmask
    CpuState::Evex, // EvexVfnmsub213phVphHphWph
    CpuState::Evex, // EvexVfnmsub213phVphHphWphKmask
    CpuState::Evex, // EvexVfnmsub231phVphHphWph
    CpuState::Evex, // EvexVfnmsub231phVphHphWphKmask
    CpuState::Evex, // EvexVfmaddsub132phVphHphWph
    CpuState::Evex, // EvexVfmaddsub132phVphHphWphKmask
    CpuState::Evex, // EvexVfmaddsub213phVphHphWph
    CpuState::Evex, // EvexVfmaddsub213phVphHphWphKmask
    CpuState::Evex, // EvexVfmaddsub231phVphHphWph
    CpuState::Evex, // EvexVfmaddsub231phVphHphWphKmask
    CpuState::Evex, // EvexVfmsubadd132phVphHphWph
    CpuState::Evex, // EvexVfmsubadd132phVphHphWphKmask
    CpuState::Evex, // EvexVfmsubadd213phVphHphWph
    CpuState::Evex, // EvexVfmsubadd213phVphHphWphKmask
    CpuState::Evex, // EvexVfmsubadd231phVphHphWph
    CpuState::Evex, // EvexVfmsubadd231phVphHphWphKmask
    CpuState::Evex, // EvexVfpclassphKgdWphIbKmask
    CpuState::Evex, // EvexVfpclassshKgbWshIbKmask
    CpuState::Evex, // EvexVucomishVshWsh
    CpuState::Evex, // EvexVcomishVshWsh
    CpuState::Evex, // EvexVcmpphKgdHphWphIb
    CpuState::Evex, // EvexVcmpshKgbHshWshIb
    CpuState::Evex, // EvexVsqrtphVphWph
    CpuState::Evex, // EvexVsqrtphVphWphKmask
    CpuState::Evex, // EvexVsqrtshVshHphWsh
    CpuState::Evex, // EvexVsqrtshVshHphWshKmask
    CpuState::Evex, // EvexVgetexpphVphWph
    CpuState::Evex, // EvexVgetexpphVphWphKmask
    CpuState::Evex, // EvexVgetexpshVshHphWsh
    CpuState::Evex, // EvexVgetexpshVshHphWshKmask
    CpuState::Evex, // EvexVmovshVshWsh
    CpuState::Evex, // EvexVmovshWshVsh
    CpuState::Evex, // EvexVmovshVshWshKmask
    CpuState::Evex, // EvexVmovshWshVshKmask
    CpuState::Evex, // EvexVmovshVshHphWsh
    CpuState::Evex, // EvexVmovshWshHphVsh
    CpuState::Evex, // EvexVmovshVshHphWshKmask
    CpuState::Evex, // EvexVmovshWshHphVshKmask
    CpuState::Evex, // EvexVmovwVshEw
    CpuState::Evex, // EvexVmovwEdVsh
    CpuState::Evex, // EvexVcvtph2uwVdqWps
    CpuState::Evex, // EvexVcvtph2uwVdqWpsKmask
    CpuState::Evex, // EvexVcvtph2wVdqWps
    CpuState::Evex, // EvexVcvtph2wVdqWpsKmask
    CpuState::Evex, // EvexVcvttph2uwVdqWps
    CpuState::Evex, // EvexVcvttph2uwVdqWpsKmask
    CpuState::Evex, // EvexVcvttph2wVdqWps
    CpuState::Evex, // EvexVcvttph2wVdqWpsKmask
    CpuState::Evex, // EvexVcvtuw2phVphWdq
    CpuState::Evex, // EvexVcvtuw2phVphWdqKmask
    CpuState::Evex, // EvexVcvtw2phVphWdq
    CpuState::Evex, // EvexVcvtw2phVphWdqKmask
    CpuState::Evex, // EvexVcvtph2psxVpsWph
    CpuState::Evex, // EvexVcvtph2psxVpsWphKmask
    CpuState::Evex, // EvexVcvtph2dqVdqWph
    CpuState::Evex, // EvexVcvtph2dqVdqWphKmask
    CpuState::Evex, // EvexVcvtph2udqVdqWph
    CpuState::Evex, // EvexVcvtph2udqVdqWphKmask
    CpuState::Evex, // EvexVcvttph2dqVdqWph
    CpuState::Evex, // EvexVcvttph2dqVdqWphKmask
    CpuState::Evex, // EvexVcvttph2udqVdqWph
    CpuState::Evex, // EvexVcvttph2udqVdqWphKmask
    CpuState::Evex, // EvexVcvtph2pdVpdWph
    CpuState::Evex, // EvexVcvtph2pdVpdWphKmask
    CpuState::Evex, // EvexVcvtph2qqVdqWph
    CpuState::Evex, // EvexVcvtph2qqVdqWphKmask
    CpuState::Evex, // EvexVcvtph2uqqVdqWph
    CpuState::Evex, // EvexVcvtph2uqqVdqWphKmask
    CpuState::Evex, // EvexVcvttph2qqVdqWph
    CpuState::Evex, // EvexVcvttph2qqVdqWphKmask
    CpuState::Evex, // EvexVcvttph2uqqVdqWph
    CpuState::Evex, // EvexVcvttph2uqqVdqWphKmask
    CpuState::Evex, // EvexVcvtps2phxVphWdq
    CpuState::Evex, // EvexVcvtps2phxVphWdqKmask
    CpuState::Evex, // EvexVcvtdq2phVphWdq
    CpuState::Evex, // EvexVcvtdq2phVphWdqKmask
    CpuState::Evex, // EvexVcvtudq2phVphWdq
    CpuState::Evex, // EvexVcvtudq2phVphWdqKmask
    CpuState::Evex, // EvexVcvtpd2phVphWdq
    CpuState::Evex, // EvexVcvtpd2phVphWdqKmask
    CpuState::Evex, // EvexVcvtqq2phVphWdq
    CpuState::Evex, // EvexVcvtqq2phVphWdqKmask
    CpuState::Evex, // EvexVcvtuqq2phVphWdq
    CpuState::Evex, // EvexVcvtuqq2phVphWdqKmask
    CpuState::Evex, // EvexVcvtsh2ssVssWsh
    CpuState::Evex, // EvexVcvtsh2ssVssWshKmask
    CpuState::Evex, // EvexVcvtsh2sdVsdWsh
    CpuState::Evex, // EvexVcvtsh2sdVsdWshKmask
    CpuState::Evex, // EvexVcvtss2shVssWsh
    CpuState::Evex, // EvexVcvtss2shVssWshKmask
    CpuState::Evex, // EvexVcvtsd2shVssWsh
    CpuState::Evex, // EvexVcvtsd2shVssWshKmask
    CpuState::Evex, // EvexVcvtsh2siGdWss
    CpuState::Evex, // EvexVcvtsh2siGqWss
    CpuState::Evex, // EvexVcvtsh2usiGdWss
    CpuState::Evex, // EvexVcvtsh2usiGqWss
    CpuState::Evex, // EvexVcvttsh2siGdWss
    CpuState::Evex, // EvexVcvttsh2siGqWss
    CpuState::Evex, // EvexVcvttsh2usiGdWss
    CpuState::Evex, // EvexVcvttsh2usiGqWss
    CpuState::Evex, // EvexVcvtsi2shVshEd
    CpuState::Evex, // EvexVcvtsi2shVshEq
    CpuState::Evex, // EvexVcvtusi2shVshEd
    CpuState::Evex, // EvexVcvtusi2shVshEq
    CpuState::Evex, // EvexVgetmantphVphWphIbKmask
    CpuState::Evex, // EvexVgetmantshVshHphWshIbKmask
    CpuState::Evex, // EvexVreducephVphWphIbKmask
    CpuState::Evex, // EvexVreduceshVshHphWshIbKmask
    CpuState::Evex, // EvexVrndscalephVphWphIbKmask
    CpuState::Evex, // EvexVrndscaleshVshHphWshIbKmask
    CpuState::Evex, // EvexVrcpphVphWphKmask
    CpuState::Evex, // EvexVrcpshVshHphWshKmask
    CpuState::Evex, // EvexVrsqrtphVphWphKmask
    CpuState::Evex, // EvexVrsqrtshVshHphWshKmask
    CpuState::Evex, // EvexVfmulcshVshHphWshKmask
    CpuState::Evex, // EvexVfcmulcshVshHphWshKmask
    CpuState::Evex, // EvexVfmulcphVphHphWphKmask
    CpuState::Evex, // EvexVfcmulcphVphHphWphKmask
    CpuState::Evex, // EvexVfmaddcshVshHphWshKmask
    CpuState::Evex, // EvexVfcmaddcshVshHphWshKmask
    CpuState::Evex, // EvexVfmaddcphVphHphWphKmask
    CpuState::Evex, // EvexVfcmaddcphVphHphWphKmask
    CpuState::Evex, // EvexVaesencVdqHdqWdq
    CpuState::Evex, // EvexVaesenclastVdqHdqWdq
    CpuState::Evex, // EvexVaesdecVdqHdqWdq
    CpuState::Evex, // EvexVaesdeclastVdqHdqWdq
    CpuState::Evex, // EvexVpclmulqdqVdqHdqWdqIb
    CpuState::Evex, // EvexVgf2p8affineqbVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVgf2p8affineinvqbVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVgf2p8mulbVdqHdqWdqKmask
    CpuState::Evex, // EvexVsm4key4VdqHdqWdq
    CpuState::Evex, // EvexVsm4rnds4VdqHdqWdq
    CpuState::Evex, // EvexVucomxssVssWss
    CpuState::Evex, // EvexVcomxssVssWss
    CpuState::Evex, // EvexVucomxsdVsdWsd
    CpuState::Evex, // EvexVcomxsdVsdWsd
    CpuState::Evex, // EvexVucomxshVshWsh
    CpuState::Evex, // EvexVcomxshVshWsh
    CpuState::Evex, // EvexVpdpbssdVdqHdqWdq
    CpuState::Evex, // EvexVpdpbssdsVdqHdqWdq
    CpuState::Evex, // EvexVpdpbsudVdqHdqWdq
    CpuState::Evex, // EvexVpdpbsudsVdqHdqWdq
    CpuState::Evex, // EvexVpdpbuudVdqHdqWdq
    CpuState::Evex, // EvexVpdpbuudsVdqHdqWdq
    CpuState::Evex, // EvexVpdpbssdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpbssdsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpbsudVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpbsudsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpbuudVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpbuudsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwsudVdqHdqWdq
    CpuState::Evex, // EvexVpdpwsudsVdqHdqWdq
    CpuState::Evex, // EvexVpdpwusdVdqHdqWdq
    CpuState::Evex, // EvexVpdpwusdsVdqHdqWdq
    CpuState::Evex, // EvexVpdpwuudVdqHdqWdq
    CpuState::Evex, // EvexVpdpwuudsVdqHdqWdq
    CpuState::Evex, // EvexVpdpwsudVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwsudsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwusdVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwusdsVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwuudVdqHdqWdqKmask
    CpuState::Evex, // EvexVpdpwuudsVdqHdqWdqKmask
    CpuState::Evex, // EvexVmpsadbwVdqHdqWdqIb
    CpuState::Evex, // EvexVmpsadbwVdqHdqWdqIbKmask
    CpuState::Evex, // EvexVdpphpsVpsHdqWdqKmask
    CpuState::Evex, // EvexVaddbf16VphHphWph
    CpuState::Evex, // EvexVaddbf16VphHphWphKmask
    CpuState::Evex, // EvexVsubbf16VphHphWph
    CpuState::Evex, // EvexVsubbf16VphHphWphKmask
    CpuState::Evex, // EvexVdivbf16VphHphWph
    CpuState::Evex, // EvexVdivbf16VphHphWphKmask
    CpuState::Evex, // EvexVmulbf16VphHphWph
    CpuState::Evex, // EvexVmulbf16VphHphWphKmask
    CpuState::Evex, // EvexVminpbf16VphHphWph
    CpuState::Evex, // EvexVminpbf16VphHphWphKmask
    CpuState::Evex, // EvexVmaxpbf16VphHphWph
    CpuState::Evex, // EvexVmaxpbf16VphHphWphKmask
    CpuState::Evex, // EvexVscalefpbf16VphHphWph
    CpuState::Evex, // EvexVscalefpbf16VphHphWphKmask
    CpuState::Evex, // EvexVsqrtbf16VphWph
    CpuState::Evex, // EvexVsqrtbf16VphWphKmask
    CpuState::Evex, // EvexVgetexppbf16VphWph
    CpuState::Evex, // EvexVgetexppbf16VphWphKmask
    CpuState::Evex, // EvexVfmadd132bf16VphHphWph
    CpuState::Evex, // EvexVfmadd132bf16VphHphWphKmask
    CpuState::Evex, // EvexVfmadd213bf16VphHphWph
    CpuState::Evex, // EvexVfmadd213bf16VphHphWphKmask
    CpuState::Evex, // EvexVfmadd231bf16VphHphWph
    CpuState::Evex, // EvexVfmadd231bf16VphHphWphKmask
    CpuState::Evex, // EvexVfmsub132bf16VphHphWph
    CpuState::Evex, // EvexVfmsub132bf16VphHphWphKmask
    CpuState::Evex, // EvexVfmsub213bf16VphHphWph
    CpuState::Evex, // EvexVfmsub213bf16VphHphWphKmask
    CpuState::Evex, // EvexVfmsub231bf16VphHphWph
    CpuState::Evex, // EvexVfmsub231bf16VphHphWphKmask
    CpuState::Evex, // EvexVfnmadd132bf16VphHphWph
    CpuState::Evex, // EvexVfnmadd132bf16VphHphWphKmask
    CpuState::Evex, // EvexVfnmadd213bf16VphHphWph
    CpuState::Evex, // EvexVfnmadd213bf16VphHphWphKmask
    CpuState::Evex, // EvexVfnmadd231bf16VphHphWph
    CpuState::Evex, // EvexVfnmadd231bf16VphHphWphKmask
    CpuState::Evex, // EvexVfnmsub132bf16VphHphWph
    CpuState::Evex, // EvexVfnmsub132bf16VphHphWphKmask
    CpuState::Evex, // EvexVfnmsub213bf16VphHphWph
    CpuState::Evex, // EvexVfnmsub213bf16VphHphWphKmask
    CpuState::Evex, // EvexVfnmsub231bf16VphHphWph
    CpuState::Evex, // EvexVfnmsub231bf16VphHphWphKmask
    CpuState::Evex, // EvexVfpclasspbf16KgdWphIbKmask
    CpuState::Evex, // EvexVcmppbf16KgdHphWphIb
    CpuState::Evex, // EvexVcomisbf16VshWsh
    CpuState::Evex, // EvexVgetmantpbf16VphWphIbKmask
    CpuState::Evex, // EvexVreducebf16VphWphIbKmask
    CpuState::Evex, // EvexVrndscalebf16VphWphIbKmask
    CpuState::Evex, // EvexVrcppbf16VphWph
    CpuState::Evex, // EvexVrcppbf16VphWphKmask
    CpuState::Evex, // EvexVrsqrtpbf16VphWph
    CpuState::Evex, // EvexVrsqrtpbf16VphWphKmask
    CpuState::Evex, // EvexVminmaxpsVpsHpsWpsIbKmask
    CpuState::Evex, // EvexVminmaxssVssHpsWssIbKmask
    CpuState::Evex, // EvexVminmaxpdVpdHpdWpdIbKmask
    CpuState::Evex, // EvexVminmaxsdVsdHpdWsdIbKmask
    CpuState::Evex, // EvexVminmaxphVphHphWphIbKmask
    CpuState::Evex, // EvexVminmaxshVshHphWshIbKmask
    CpuState::Evex, // EvexVminmaxbf16VphHphWphIbKmask
    CpuState::Evex, // EvexVcvt2ps2phxVphHpsWpsKmask
    CpuState::Evex, // EvexVcvttps2qqsVdqWps
    CpuState::Evex, // EvexVcvttps2qqsVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2qqsVdqWpd
    CpuState::Evex, // EvexVcvttpd2qqsVdqWpdKmask
    CpuState::Evex, // EvexVcvttps2uqqsVdqWps
    CpuState::Evex, // EvexVcvttps2uqqsVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2uqqsVdqWpd
    CpuState::Evex, // EvexVcvttpd2uqqsVdqWpdKmask
    CpuState::Evex, // EvexVcvttps2dqsVdqWps
    CpuState::Evex, // EvexVcvttps2dqsVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2dqsVdqWpd
    CpuState::Evex, // EvexVcvttpd2dqsVdqWpdKmask
    CpuState::Evex, // EvexVcvttps2udqsVdqWps
    CpuState::Evex, // EvexVcvttpd2udqsVdqWpd
    CpuState::Evex, // EvexVcvttps2udqsVdqWpsKmask
    CpuState::Evex, // EvexVcvttpd2udqsVdqWpdKmask
    CpuState::Evex, // EvexVcvttss2sisGdWss
    CpuState::Evex, // EvexVcvttss2sisGqWss
    CpuState::Evex, // EvexVcvttsd2sisGdWsd
    CpuState::Evex, // EvexVcvttsd2sisGqWsd
    CpuState::Evex, // EvexVcvttss2usisGdWss
    CpuState::Evex, // EvexVcvttss2usisGqWss
    CpuState::Evex, // EvexVcvttsd2usisGdWsd
    CpuState::Evex, // EvexVcvttsd2usisGqWsd
    CpuState::Evex, // EvexVmovwVshWsh
    CpuState::Evex, // EvexVmovwWshVsh
    CpuState::Evex, // EvexVmovdVdWd
    CpuState::Evex, // EvexVmovdWdVd
    CpuState::Evex, // EvexVcvthf82phVphWf8Kmask
    CpuState::Evex, // EvexVcvtph2bf8Vf8hdqWphKmask
    CpuState::Evex, // EvexVcvtph2bf8sVf8hdqWphKmask
    CpuState::Evex, // EvexVcvt2ph2bf8Vf8hdqWphKmask
    CpuState::Evex, // EvexVcvt2ph2bf8sVf8hdqWphKmask
    CpuState::Evex, // EvexVcvtbiasph2bf8Vf8hdqWphKmask
    CpuState::Evex, // EvexVcvtbiasph2bf8sVf8hdqWphKmask
    CpuState::Evex, // EvexVcvtph2hf8Vf8hdqWphKmask
    CpuState::Evex, // EvexVcvtph2hf8sVf8hdqWphKmask
    CpuState::Evex, // EvexVcvt2ph2hf8Vf8hdqWphKmask
    CpuState::Evex, // EvexVcvt2ph2hf8sVf8hdqWphKmask
    CpuState::Evex, // EvexVcvtbiasph2hf8Vf8hdqWphKmask
    CpuState::Evex, // EvexVcvtbiasph2hf8sVf8hdqWphKmask
    CpuState::Evex, // EvexVcvtbf162ibsV8bWph
    CpuState::Evex, // EvexVcvtbf162ibsV8bWphKmask
    CpuState::Evex, // EvexVcvtbf162iubsV8bWph
    CpuState::Evex, // EvexVcvtbf162iubsV8bWphKmask
    CpuState::Evex, // EvexVcvttbf162ibsV8bWph
    CpuState::Evex, // EvexVcvttbf162ibsV8bWphKmask
    CpuState::Evex, // EvexVcvttbf162iubsV8bWph
    CpuState::Evex, // EvexVcvttbf162iubsV8bWphKmask
    CpuState::Evex, // EvexVcvtph2ibsV8bWph
    CpuState::Evex, // EvexVcvtph2ibsV8bWphKmask
    CpuState::Evex, // EvexVcvtph2iubsV8bWph
    CpuState::Evex, // EvexVcvtph2iubsV8bWphKmask
    CpuState::Evex, // EvexVcvttph2ibsV8bWph
    CpuState::Evex, // EvexVcvttph2ibsV8bWphKmask
    CpuState::Evex, // EvexVcvttph2iubsV8bWph
    CpuState::Evex, // EvexVcvttph2iubsV8bWphKmask
    CpuState::Evex, // EvexVcvtps2ibsV8bWps
    CpuState::Evex, // EvexVcvtps2ibsV8bWpsKmask
    CpuState::Evex, // EvexVcvtps2iubsV8bWps
    CpuState::Evex, // EvexVcvtps2iubsV8bWpsKmask
    CpuState::Evex, // EvexVcvttps2ibsV8bWps
    CpuState::Evex, // EvexVcvttps2ibsV8bWpsKmask
    CpuState::Evex, // EvexVcvttps2iubsV8bWps
    CpuState::Evex, // EvexVcvttps2iubsV8bWpsKmask
    CpuState::Amx, // EvexTilemovrowVdqTrmIb
    CpuState::Amx, // EvexTilemovrowVdqTrmBd
    CpuState::Amx, // EvexTcvtrowd2psVpsTrmIb
    CpuState::Amx, // EvexTcvtrowd2psVpsTrmBd
    CpuState::Amx, // EvexTcvtrowps2phlVphTrmIb
    CpuState::Amx, // EvexTcvtrowps2phlVphTrmBd
    CpuState::Amx, // EvexTcvtrowps2phhVphTrmIb
    CpuState::Amx, // EvexTcvtrowps2phhVphTrmBd
    CpuState::Amx, // EvexTcvtrowps2bf16lVphTrmIb
    CpuState::Amx, // EvexTcvtrowps2bf16lVphTrmBd
    CpuState::Amx, // EvexTcvtrowps2bf16hVphTrmIb
    CpuState::Amx, // EvexTcvtrowps2bf16hVphTrmBd
    CpuState::Evex, // EvexVmovrsbVdqWdq
    CpuState::Evex, // EvexVmovrsbVdqWdqKmask
    CpuState::Evex, // EvexVmovrswVdqWdq
    CpuState::Evex, // EvexVmovrswVdqWdqKmask
    CpuState::Evex, // EvexVmovrsdVdqWdq
    CpuState::Evex, // EvexVmovrsdVdqWdqKmask
    CpuState::Evex, // EvexVmovrsqVdqWdq
    CpuState::Evex, // EvexVmovrsqVdqWdqKmask
    CpuState::Base, // NoAvxState
    CpuState::Base, // NoEvexState
];

/// CPU state `opcode` requires before it may execute.
#[inline]
pub const fn opcode_state(opcode: Opcode) -> CpuState {
    OPCODE_STATE[opcode as usize]
}

/// Opcodes requiring AVX state, pinned by tests so a regeneration that
/// silently drops the gate is caught.
pub const STATE_AVX_OPCODE_COUNT: usize = 676;

/// Opcodes requiring AVX-512 state.
pub const STATE_EVEX_OPCODE_COUNT: usize = 1384;

#[allow(dead_code)]
fn _feature_type_is_used(f: X86Feature) -> u16 {
    // Keeps the X86Feature import meaningful: the table stores raw
    // discriminants of exactly this enum.
    f as u16
}
