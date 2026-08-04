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

// Required CPU state — the BX_PREPARE_* attribute of
// `bx_define_opcode`. Bochs consults it in `assignHandler` and swaps
// the handler for BxNoFPU/BxNoMMX/BxNoSSE/BxNoAVX/BxNoEVEX when the
// state is unavailable; rusty_box applies it at icache fill, so the
// dispatch loop pays nothing and the check cannot be forgotten by an
// individual handler.
/// No CPU-state requirement beyond the base ISA.
pub const STATE_NONE: u8 = 0;
/// Needs x87 state (CR0.EM/TS).
pub const STATE_FPU: u8 = 1;
/// Needs MMX state.
pub const STATE_MMX: u8 = 2;
/// Needs SSE state (CR0.EM, CR4.OSFXSR, CR0.TS).
pub const STATE_SSE: u8 = 3;
/// Needs AVX state (protected mode, CR4.OSXSAVE, XCR0.SSE|YMM).
pub const STATE_AVX: u8 = 4;
/// Needs AVX-512 state (AVX plus XCR0.OPMASK|ZMM_HI256|HI_ZMM).
pub const STATE_EVEX: u8 = 5;
/// Needs AMX state.
pub const STATE_AMX: u8 = 6;

/// CPU state each opcode requires, from field 10 of `bx_define_opcode`.
// A `const` for the same reason as OPCODE_EVEX_FLAGS.
pub const OPCODE_PREPARE: [u8; 3679] = [
    0, // IaError -> STATE_NONE
    0, // InsertedOpcode -> STATE_NONE
    0, // Aaa -> STATE_NONE
    0, // Aad -> STATE_NONE
    0, // Aam -> STATE_NONE
    0, // Aas -> STATE_NONE
    0, // Daa -> STATE_NONE
    0, // Das -> STATE_NONE
    0, // AdcEbGb -> STATE_NONE
    0, // AndEbGb -> STATE_NONE
    0, // AddEbGb -> STATE_NONE
    0, // CmpEbGb -> STATE_NONE
    0, // OrEbGb -> STATE_NONE
    0, // SbbEbGb -> STATE_NONE
    0, // SubEbGb -> STATE_NONE
    0, // TestEbGb -> STATE_NONE
    0, // XorEbGb -> STATE_NONE
    0, // AdcEwGw -> STATE_NONE
    0, // AddEwGw -> STATE_NONE
    0, // AndEwGw -> STATE_NONE
    0, // CmpEwGw -> STATE_NONE
    0, // OrEwGw -> STATE_NONE
    0, // SbbEwGw -> STATE_NONE
    0, // SubEwGw -> STATE_NONE
    0, // TestEwGw -> STATE_NONE
    0, // XorEwGw -> STATE_NONE
    0, // AdcEdGd -> STATE_NONE
    0, // AddEdGd -> STATE_NONE
    0, // AndEdGd -> STATE_NONE
    0, // CmpEdGd -> STATE_NONE
    0, // OrEdGd -> STATE_NONE
    0, // SbbEdGd -> STATE_NONE
    0, // SubEdGd -> STATE_NONE
    0, // TestEdGd -> STATE_NONE
    0, // XorEdGd -> STATE_NONE
    0, // AdcAlib -> STATE_NONE
    0, // AddAlib -> STATE_NONE
    0, // AndAlib -> STATE_NONE
    0, // CmpAlib -> STATE_NONE
    0, // OrAlib -> STATE_NONE
    0, // SbbAlib -> STATE_NONE
    0, // SubAlib -> STATE_NONE
    0, // TestAlib -> STATE_NONE
    0, // XorAlib -> STATE_NONE
    0, // AdcAxiw -> STATE_NONE
    0, // AddAxiw -> STATE_NONE
    0, // AndAxiw -> STATE_NONE
    0, // CmpAxiw -> STATE_NONE
    0, // OrAxiw -> STATE_NONE
    0, // SbbAxiw -> STATE_NONE
    0, // SubAxiw -> STATE_NONE
    0, // TestAxiw -> STATE_NONE
    0, // XorAxiw -> STATE_NONE
    0, // AdcEaxid -> STATE_NONE
    0, // AddEaxid -> STATE_NONE
    0, // AndEaxid -> STATE_NONE
    0, // CmpEaxid -> STATE_NONE
    0, // OrEaxid -> STATE_NONE
    0, // SbbEaxid -> STATE_NONE
    0, // SubEaxid -> STATE_NONE
    0, // TestEaxid -> STATE_NONE
    0, // XorEaxid -> STATE_NONE
    0, // AddEbIb -> STATE_NONE
    0, // OrEbIb -> STATE_NONE
    0, // AdcEbIb -> STATE_NONE
    0, // SbbEbIb -> STATE_NONE
    0, // AndEbIb -> STATE_NONE
    0, // SubEbIb -> STATE_NONE
    0, // XorEbIb -> STATE_NONE
    0, // TestEbIb -> STATE_NONE
    0, // CmpEbIb -> STATE_NONE
    0, // AddEwIw -> STATE_NONE
    0, // OrEwIw -> STATE_NONE
    0, // AdcEwIw -> STATE_NONE
    0, // SbbEwIw -> STATE_NONE
    0, // AndEwIw -> STATE_NONE
    0, // SubEwIw -> STATE_NONE
    0, // XorEwIw -> STATE_NONE
    0, // TestEwIw -> STATE_NONE
    0, // CmpEwIw -> STATE_NONE
    0, // AddEwsIb -> STATE_NONE
    0, // OrEwsIb -> STATE_NONE
    0, // AdcEwsIb -> STATE_NONE
    0, // SbbEwsIb -> STATE_NONE
    0, // AndEwsIb -> STATE_NONE
    0, // SubEwsIb -> STATE_NONE
    0, // XorEwsIb -> STATE_NONE
    0, // TestEwsIb -> STATE_NONE
    0, // CmpEwsIb -> STATE_NONE
    0, // AddEdId -> STATE_NONE
    0, // OrEdId -> STATE_NONE
    0, // AdcEdId -> STATE_NONE
    0, // SbbEdId -> STATE_NONE
    0, // AndEdId -> STATE_NONE
    0, // SubEdId -> STATE_NONE
    0, // XorEdId -> STATE_NONE
    0, // TestEdId -> STATE_NONE
    0, // CmpEdId -> STATE_NONE
    0, // AddEdsIb -> STATE_NONE
    0, // OrEdsIb -> STATE_NONE
    0, // AdcEdsIb -> STATE_NONE
    0, // SbbEdsIb -> STATE_NONE
    0, // AndEdsIb -> STATE_NONE
    0, // SubEdsIb -> STATE_NONE
    0, // XorEdsIb -> STATE_NONE
    0, // TestEdsIb -> STATE_NONE
    0, // CmpEdsIb -> STATE_NONE
    0, // XorEwGwZeroIdiom -> STATE_NONE
    0, // XorGwEwZeroIdiom -> STATE_NONE
    0, // XorEdGdZeroIdiom -> STATE_NONE
    0, // XorGdEdZeroIdiom -> STATE_NONE
    0, // SubEwGwZeroIdiom -> STATE_NONE
    0, // SubGwEwZeroIdiom -> STATE_NONE
    0, // SubEdGdZeroIdiom -> STATE_NONE
    0, // SubGdEdZeroIdiom -> STATE_NONE
    0, // AddGbEb -> STATE_NONE
    0, // OrGbEb -> STATE_NONE
    0, // AdcGbEb -> STATE_NONE
    0, // SbbGbEb -> STATE_NONE
    0, // AndGbEb -> STATE_NONE
    0, // SubGbEb -> STATE_NONE
    0, // XorGbEb -> STATE_NONE
    0, // CmpGbEb -> STATE_NONE
    0, // AdcGwEw -> STATE_NONE
    0, // AddGwEw -> STATE_NONE
    0, // AndGwEw -> STATE_NONE
    0, // CmpGwEw -> STATE_NONE
    0, // OrGwEw -> STATE_NONE
    0, // SbbGwEw -> STATE_NONE
    0, // SubGwEw -> STATE_NONE
    0, // XorGwEw -> STATE_NONE
    0, // AdcGdEd -> STATE_NONE
    0, // AddGdEd -> STATE_NONE
    0, // AndGdEd -> STATE_NONE
    0, // CmpGdEd -> STATE_NONE
    0, // OrGdEd -> STATE_NONE
    0, // SbbGdEd -> STATE_NONE
    0, // SubGdEd -> STATE_NONE
    0, // XorGdEd -> STATE_NONE
    0, // IncEb -> STATE_NONE
    0, // IncEw -> STATE_NONE
    0, // IncEd -> STATE_NONE
    0, // DecEb -> STATE_NONE
    0, // DecEw -> STATE_NONE
    0, // DecEd -> STATE_NONE
    0, // BsfGwEw -> STATE_NONE
    0, // BsrGwEw -> STATE_NONE
    0, // BsfGdEd -> STATE_NONE
    0, // BsrGdEd -> STATE_NONE
    0, // BtcEwGw -> STATE_NONE
    0, // BtrEwGw -> STATE_NONE
    0, // BtsEwGw -> STATE_NONE
    0, // BtcEdGd -> STATE_NONE
    0, // BtrEdGd -> STATE_NONE
    0, // BtsEdGd -> STATE_NONE
    0, // BtcEwIb -> STATE_NONE
    0, // BtrEwIb -> STATE_NONE
    0, // BtsEwIb -> STATE_NONE
    0, // BtcEdIb -> STATE_NONE
    0, // BtrEdIb -> STATE_NONE
    0, // BtsEdIb -> STATE_NONE
    0, // BtEwIb -> STATE_NONE
    0, // BtEdIb -> STATE_NONE
    0, // BtEwGw -> STATE_NONE
    0, // BtEdGd -> STATE_NONE
    0, // BoundGwMa -> STATE_NONE
    0, // BoundGdMa -> STATE_NONE
    0, // ArplEwGw -> STATE_NONE
    0, // CallEd -> STATE_NONE
    0, // CallEw -> STATE_NONE
    0, // CallJd -> STATE_NONE
    0, // CallJw -> STATE_NONE
    0, // CallfOp16Ap -> STATE_NONE
    0, // CallfOp32Ap -> STATE_NONE
    0, // CallfOp16Ep -> STATE_NONE
    0, // CallfOp32Ep -> STATE_NONE
    0, // Cbw -> STATE_NONE
    0, // Cdq -> STATE_NONE
    0, // Cwd -> STATE_NONE
    0, // Cwde -> STATE_NONE
    0, // Clc -> STATE_NONE
    0, // Cld -> STATE_NONE
    0, // Cli -> STATE_NONE
    0, // Clts -> STATE_NONE
    0, // Cmc -> STATE_NONE
    0, // Hlt -> STATE_NONE
    0, // Clflush -> STATE_NONE
    0, // Clflushopt -> STATE_NONE
    0, // Clwb -> STATE_NONE
    0, // Clzero -> STATE_NONE
    0, // EnterOp16IwIb -> STATE_NONE
    0, // EnterOp32IwIb -> STATE_NONE
    0, // LeaveOp16 -> STATE_NONE
    0, // LeaveOp32 -> STATE_NONE
    0, // ImulGdEd -> STATE_NONE
    0, // ImulGdEdId -> STATE_NONE
    0, // ImulGdEdsIb -> STATE_NONE
    0, // ImulGwEw -> STATE_NONE
    0, // ImulGwEwIw -> STATE_NONE
    0, // ImulGwEwsIb -> STATE_NONE
    0, // InAlDx -> STATE_NONE
    0, // InAlib -> STATE_NONE
    0, // InAxDx -> STATE_NONE
    0, // InAxib -> STATE_NONE
    0, // InEaxDx -> STATE_NONE
    0, // InEaxib -> STATE_NONE
    0, // OutDxAl -> STATE_NONE
    0, // OutDxAx -> STATE_NONE
    0, // OutDxEax -> STATE_NONE
    0, // OutIbAl -> STATE_NONE
    0, // OutIbAx -> STATE_NONE
    0, // OutIbEax -> STATE_NONE
    0, // IntIb -> STATE_NONE
    0, // INT1 -> STATE_NONE
    0, // INT3 -> STATE_NONE
    0, // Int0 -> STATE_NONE
    0, // IretOp16 -> STATE_NONE
    0, // IretOp32 -> STATE_NONE
    0, // JmpEd -> STATE_NONE
    0, // JmpEw -> STATE_NONE
    0, // JmpJw -> STATE_NONE
    0, // JmpJbw -> STATE_NONE
    0, // JmpJd -> STATE_NONE
    0, // JmpJbd -> STATE_NONE
    0, // JmpfAp -> STATE_NONE
    0, // JmpfOp16Ep -> STATE_NONE
    0, // JmpfOp32Ep -> STATE_NONE
    0, // JcxzJbw -> STATE_NONE
    0, // JecxzJbd -> STATE_NONE
    0, // LoopJbw -> STATE_NONE
    0, // LoopeJbw -> STATE_NONE
    0, // LoopneJbw -> STATE_NONE
    0, // LoopJbd -> STATE_NONE
    0, // LoopeJbd -> STATE_NONE
    0, // LoopneJbd -> STATE_NONE
    0, // JbJw -> STATE_NONE
    0, // JbeJw -> STATE_NONE
    0, // JlJw -> STATE_NONE
    0, // JleJw -> STATE_NONE
    0, // JnbJw -> STATE_NONE
    0, // JnbeJw -> STATE_NONE
    0, // JnlJw -> STATE_NONE
    0, // JnleJw -> STATE_NONE
    0, // JnoJw -> STATE_NONE
    0, // JnpJw -> STATE_NONE
    0, // JnsJw -> STATE_NONE
    0, // JnzJw -> STATE_NONE
    0, // JoJw -> STATE_NONE
    0, // JpJw -> STATE_NONE
    0, // JsJw -> STATE_NONE
    0, // JzJw -> STATE_NONE
    0, // JbJbw -> STATE_NONE
    0, // JbeJbw -> STATE_NONE
    0, // JlJbw -> STATE_NONE
    0, // JleJbw -> STATE_NONE
    0, // JnbJbw -> STATE_NONE
    0, // JnbeJbw -> STATE_NONE
    0, // JnlJbw -> STATE_NONE
    0, // JnleJbw -> STATE_NONE
    0, // JnoJbw -> STATE_NONE
    0, // JnpJbw -> STATE_NONE
    0, // JnsJbw -> STATE_NONE
    0, // JnzJbw -> STATE_NONE
    0, // JoJbw -> STATE_NONE
    0, // JpJbw -> STATE_NONE
    0, // JsJbw -> STATE_NONE
    0, // JzJbw -> STATE_NONE
    0, // JbJd -> STATE_NONE
    0, // JbeJd -> STATE_NONE
    0, // JlJd -> STATE_NONE
    0, // JleJd -> STATE_NONE
    0, // JnbJd -> STATE_NONE
    0, // JnbeJd -> STATE_NONE
    0, // JnlJd -> STATE_NONE
    0, // JnleJd -> STATE_NONE
    0, // JnoJd -> STATE_NONE
    0, // JnpJd -> STATE_NONE
    0, // JnsJd -> STATE_NONE
    0, // JnzJd -> STATE_NONE
    0, // JoJd -> STATE_NONE
    0, // JpJd -> STATE_NONE
    0, // JsJd -> STATE_NONE
    0, // JzJd -> STATE_NONE
    0, // JbJbd -> STATE_NONE
    0, // JbeJbd -> STATE_NONE
    0, // JlJbd -> STATE_NONE
    0, // JleJbd -> STATE_NONE
    0, // JnbJbd -> STATE_NONE
    0, // JnbeJbd -> STATE_NONE
    0, // JnlJbd -> STATE_NONE
    0, // JnleJbd -> STATE_NONE
    0, // JnoJbd -> STATE_NONE
    0, // JnpJbd -> STATE_NONE
    0, // JnsJbd -> STATE_NONE
    0, // JnzJbd -> STATE_NONE
    0, // JoJbd -> STATE_NONE
    0, // JpJbd -> STATE_NONE
    0, // JsJbd -> STATE_NONE
    0, // JzJbd -> STATE_NONE
    0, // Sahf -> STATE_NONE
    0, // Lahf -> STATE_NONE
    0, // LdsGdMp -> STATE_NONE
    0, // LdsGwMp -> STATE_NONE
    0, // LesGdMp -> STATE_NONE
    0, // LesGwMp -> STATE_NONE
    0, // LfsGdMp -> STATE_NONE
    0, // LfsGwMp -> STATE_NONE
    0, // LssGdMp -> STATE_NONE
    0, // LssGwMp -> STATE_NONE
    0, // LgsGdMp -> STATE_NONE
    0, // LgsGwMp -> STATE_NONE
    0, // LarGwEw -> STATE_NONE
    0, // LslGwEw -> STATE_NONE
    0, // LarGdEw -> STATE_NONE
    0, // LslGdEw -> STATE_NONE
    0, // LeaGdM -> STATE_NONE
    0, // LeaGwM -> STATE_NONE
    0, // SidtMs -> STATE_NONE
    0, // LidtMs -> STATE_NONE
    0, // SgdtMs -> STATE_NONE
    0, // LgdtMs -> STATE_NONE
    0, // SldtEw -> STATE_NONE
    0, // LldtEw -> STATE_NONE
    0, // StrEw -> STATE_NONE
    0, // LtrEw -> STATE_NONE
    0, // SmswEw -> STATE_NONE
    0, // LmswEw -> STATE_NONE
    0, // MovCr0rd -> STATE_NONE
    0, // MovCr2rd -> STATE_NONE
    0, // MovCr3rd -> STATE_NONE
    0, // MovCr4rd -> STATE_NONE
    0, // MovRdCr0 -> STATE_NONE
    0, // MovRdCr2 -> STATE_NONE
    0, // MovRdCr3 -> STATE_NONE
    0, // MovRdCr4 -> STATE_NONE
    0, // MovRdDd -> STATE_NONE
    0, // MovDdRd -> STATE_NONE
    0, // MovEbIb -> STATE_NONE
    0, // MovEdId -> STATE_NONE
    0, // MovEwIw -> STATE_NONE
    0, // MovGbEb -> STATE_NONE
    0, // MovEbGb -> STATE_NONE
    0, // MovGwEw -> STATE_NONE
    0, // MovEwGw -> STATE_NONE
    0, // MovOp32GdEd -> STATE_NONE
    0, // MovOp32EdGd -> STATE_NONE
    0, // MovEwSw -> STATE_NONE
    0, // MovSwEw -> STATE_NONE
    0, // MovAlod -> STATE_NONE
    0, // MovAxod -> STATE_NONE
    0, // MovEaxod -> STATE_NONE
    0, // MovOdAl -> STATE_NONE
    0, // MovOdAx -> STATE_NONE
    0, // MovOdEax -> STATE_NONE
    0, // MovsxGdEb -> STATE_NONE
    0, // MovsxGdEw -> STATE_NONE
    0, // MovsxGwEb -> STATE_NONE
    0, // MovzxGdEb -> STATE_NONE
    0, // MovzxGdEw -> STATE_NONE
    0, // MovzxGwEb -> STATE_NONE
    0, // Nop -> STATE_NONE
    0, // Pause -> STATE_NONE
    0, // PopEw -> STATE_NONE
    0, // PopEd -> STATE_NONE
    0, // PopOp16Sw -> STATE_NONE
    0, // PopOp32Sw -> STATE_NONE
    0, // PopaOp16 -> STATE_NONE
    0, // PopaOp32 -> STATE_NONE
    0, // PopfFw -> STATE_NONE
    0, // PopfFd -> STATE_NONE
    0, // PushEw -> STATE_NONE
    0, // PushEd -> STATE_NONE
    0, // PushId -> STATE_NONE
    0, // PushSIb32 -> STATE_NONE
    0, // PushIw -> STATE_NONE
    0, // PushSIb16 -> STATE_NONE
    0, // PushOp16Sw -> STATE_NONE
    0, // PushOp32Sw -> STATE_NONE
    0, // PushaOp16 -> STATE_NONE
    0, // PushaOp32 -> STATE_NONE
    0, // PushfFw -> STATE_NONE
    0, // PushfFd -> STATE_NONE
    0, // RepCmpsbXbYb -> STATE_NONE
    0, // RepCmpsdXdYd -> STATE_NONE
    0, // RepCmpswXwYw -> STATE_NONE
    0, // RepInsbYbDx -> STATE_NONE
    0, // RepInsdYdDx -> STATE_NONE
    0, // RepInswYwDx -> STATE_NONE
    0, // RepLodsbAlxb -> STATE_NONE
    0, // RepLodsdEaxxd -> STATE_NONE
    0, // RepLodswAxxw -> STATE_NONE
    0, // RepMovsbYbXb -> STATE_NONE
    0, // RepMovsdYdXd -> STATE_NONE
    0, // RepMovswYwXw -> STATE_NONE
    0, // RepOutsbDxxb -> STATE_NONE
    0, // RepOutsdDxxd -> STATE_NONE
    0, // RepOutswDxxw -> STATE_NONE
    0, // RepScasbAlyb -> STATE_NONE
    0, // RepScasdEaxyd -> STATE_NONE
    0, // RepScaswAxyw -> STATE_NONE
    0, // RepStosbYbAl -> STATE_NONE
    0, // RepStosdYdEax -> STATE_NONE
    0, // RepStoswYwAx -> STATE_NONE
    0, // RetfOp16 -> STATE_NONE
    0, // RetfOp16Iw -> STATE_NONE
    0, // RetfOp32 -> STATE_NONE
    0, // RetfOp32Iw -> STATE_NONE
    0, // RetOp16 -> STATE_NONE
    0, // RetOp16Iw -> STATE_NONE
    0, // RetOp32 -> STATE_NONE
    0, // RetOp32Iw -> STATE_NONE
    0, // NotEb -> STATE_NONE
    0, // NegEb -> STATE_NONE
    0, // NotEw -> STATE_NONE
    0, // NegEw -> STATE_NONE
    0, // NotEd -> STATE_NONE
    0, // NegEd -> STATE_NONE
    0, // RolEb -> STATE_NONE
    0, // RorEb -> STATE_NONE
    0, // RclEb -> STATE_NONE
    0, // RcrEb -> STATE_NONE
    0, // ShlEb -> STATE_NONE
    0, // ShrEb -> STATE_NONE
    0, // SarEb -> STATE_NONE
    0, // RolEw -> STATE_NONE
    0, // RorEw -> STATE_NONE
    0, // RclEw -> STATE_NONE
    0, // RcrEw -> STATE_NONE
    0, // ShlEw -> STATE_NONE
    0, // ShrEw -> STATE_NONE
    0, // SarEw -> STATE_NONE
    0, // RolEd -> STATE_NONE
    0, // RorEd -> STATE_NONE
    0, // RclEd -> STATE_NONE
    0, // RcrEd -> STATE_NONE
    0, // ShlEd -> STATE_NONE
    0, // ShrEd -> STATE_NONE
    0, // SarEd -> STATE_NONE
    0, // RolEbIb -> STATE_NONE
    0, // RorEbIb -> STATE_NONE
    0, // RclEbIb -> STATE_NONE
    0, // RcrEbIb -> STATE_NONE
    0, // ShlEbIb -> STATE_NONE
    0, // ShrEbIb -> STATE_NONE
    0, // SarEbIb -> STATE_NONE
    0, // RolEwIb -> STATE_NONE
    0, // RorEwIb -> STATE_NONE
    0, // RclEwIb -> STATE_NONE
    0, // RcrEwIb -> STATE_NONE
    0, // ShlEwIb -> STATE_NONE
    0, // ShrEwIb -> STATE_NONE
    0, // SarEwIb -> STATE_NONE
    0, // RolEdIb -> STATE_NONE
    0, // RorEdIb -> STATE_NONE
    0, // RclEdIb -> STATE_NONE
    0, // RcrEdIb -> STATE_NONE
    0, // ShlEdIb -> STATE_NONE
    0, // ShrEdIb -> STATE_NONE
    0, // SarEdIb -> STATE_NONE
    0, // RolEbI1 -> STATE_NONE
    0, // RorEbI1 -> STATE_NONE
    0, // RclEbI1 -> STATE_NONE
    0, // RcrEbI1 -> STATE_NONE
    0, // ShlEbI1 -> STATE_NONE
    0, // ShrEbI1 -> STATE_NONE
    0, // SarEbI1 -> STATE_NONE
    0, // RolEwI1 -> STATE_NONE
    0, // RorEwI1 -> STATE_NONE
    0, // RclEwI1 -> STATE_NONE
    0, // RcrEwI1 -> STATE_NONE
    0, // ShlEwI1 -> STATE_NONE
    0, // ShrEwI1 -> STATE_NONE
    0, // SarEwI1 -> STATE_NONE
    0, // RolEdI1 -> STATE_NONE
    0, // RorEdI1 -> STATE_NONE
    0, // RclEdI1 -> STATE_NONE
    0, // RcrEdI1 -> STATE_NONE
    0, // ShlEdI1 -> STATE_NONE
    0, // ShrEdI1 -> STATE_NONE
    0, // SarEdI1 -> STATE_NONE
    0, // SetbEb -> STATE_NONE
    0, // SetbeEb -> STATE_NONE
    0, // SetlEb -> STATE_NONE
    0, // SetleEb -> STATE_NONE
    0, // SetnbEb -> STATE_NONE
    0, // SetnbeEb -> STATE_NONE
    0, // SetnlEb -> STATE_NONE
    0, // SetnleEb -> STATE_NONE
    0, // SetnoEb -> STATE_NONE
    0, // SetnpEb -> STATE_NONE
    0, // SetnsEb -> STATE_NONE
    0, // SetnzEb -> STATE_NONE
    0, // SetoEb -> STATE_NONE
    0, // SetpEb -> STATE_NONE
    0, // SetsEb -> STATE_NONE
    0, // SetzEb -> STATE_NONE
    0, // ShldEdGd -> STATE_NONE
    0, // ShldEdGdIb -> STATE_NONE
    0, // ShldEwGw -> STATE_NONE
    0, // ShldEwGwIb -> STATE_NONE
    0, // ShrdEdGd -> STATE_NONE
    0, // ShrdEdGdIb -> STATE_NONE
    0, // ShrdEwGw -> STATE_NONE
    0, // ShrdEwGwIb -> STATE_NONE
    0, // Rsm -> STATE_NONE
    0, // Salc -> STATE_NONE
    0, // Stc -> STATE_NONE
    0, // Std -> STATE_NONE
    0, // Sti -> STATE_NONE
    0, // MulAleb -> STATE_NONE
    0, // ImulAleb -> STATE_NONE
    0, // DivAleb -> STATE_NONE
    0, // IdivAleb -> STATE_NONE
    0, // MulAxew -> STATE_NONE
    0, // ImulAxew -> STATE_NONE
    0, // DivAxew -> STATE_NONE
    0, // IdivAxew -> STATE_NONE
    0, // MulEaxed -> STATE_NONE
    0, // ImulEaxed -> STATE_NONE
    0, // DivEaxed -> STATE_NONE
    0, // IdivEaxed -> STATE_NONE
    0, // VerrEw -> STATE_NONE
    0, // VerwEw -> STATE_NONE
    0, // XchgEbGb -> STATE_NONE
    0, // XchgEwGw -> STATE_NONE
    0, // XchgEdGd -> STATE_NONE
    0, // XchgRxax -> STATE_NONE
    0, // XchgErxEax -> STATE_NONE
    0, // Xlat -> STATE_NONE
    0, // Sysenter -> STATE_NONE
    0, // Sysexit -> STATE_NONE
    0, // Monitor -> STATE_NONE
    0, // Mwait -> STATE_NONE
    0, // UmonitorEq -> STATE_NONE
    0, // UmonitorEd -> STATE_NONE
    0, // UmwaitEd -> STATE_NONE
    0, // TpauseEd -> STATE_NONE
    0, // Monitorx -> STATE_NONE
    0, // Mwaitx -> STATE_NONE
    0, // Fwait -> STATE_NONE
    1, // FldSti -> STATE_FPU
    1, // FldSingleReal -> STATE_FPU
    1, // FldDoubleReal -> STATE_FPU
    1, // FldExtendedReal -> STATE_FPU
    1, // FildWordInteger -> STATE_FPU
    1, // FildDwordInteger -> STATE_FPU
    1, // FildQwordInteger -> STATE_FPU
    1, // FbldPackedBcd -> STATE_FPU
    1, // FstSti -> STATE_FPU
    1, // FstpSti -> STATE_FPU
    1, // FstpSpecialSti -> STATE_FPU
    1, // FstSingleReal -> STATE_FPU
    1, // FstpSingleReal -> STATE_FPU
    1, // FstDoubleReal -> STATE_FPU
    1, // FstpDoubleReal -> STATE_FPU
    1, // FstpExtendedReal -> STATE_FPU
    1, // FistWordInteger -> STATE_FPU
    1, // FistpWordInteger -> STATE_FPU
    1, // FistDwordInteger -> STATE_FPU
    1, // FistpDwordInteger -> STATE_FPU
    1, // FistpQwordInteger -> STATE_FPU
    1, // FbstpPackedBcd -> STATE_FPU
    1, // FisttpMw -> STATE_FPU
    1, // FisttpMd -> STATE_FPU
    1, // FisttpMq -> STATE_FPU
    1, // Fninit -> STATE_FPU
    1, // Fnclex -> STATE_FPU
    1, // Frstor -> STATE_FPU
    1, // Fnsave -> STATE_FPU
    1, // Fldenv -> STATE_FPU
    1, // Fnstenv -> STATE_FPU
    1, // Fldcw -> STATE_FPU
    1, // Fnstcw -> STATE_FPU
    1, // Fnstsw -> STATE_FPU
    1, // FnstswAx -> STATE_FPU
    1, // FLD1 -> STATE_FPU
    1, // Fldl2t -> STATE_FPU
    1, // Fldl2e -> STATE_FPU
    1, // Fldpi -> STATE_FPU
    1, // Fldlg2 -> STATE_FPU
    1, // Fldln2 -> STATE_FPU
    1, // Fldz -> STATE_FPU
    1, // FaddSt0Stj -> STATE_FPU
    1, // FaddStiSt0 -> STATE_FPU
    1, // FaddpStiSt0 -> STATE_FPU
    1, // FaddSingleReal -> STATE_FPU
    1, // FaddDoubleReal -> STATE_FPU
    1, // FiaddWordInteger -> STATE_FPU
    1, // FiaddDwordInteger -> STATE_FPU
    1, // FmulSt0Stj -> STATE_FPU
    1, // FmulStiSt0 -> STATE_FPU
    1, // FmulpStiSt0 -> STATE_FPU
    1, // FmulSingleReal -> STATE_FPU
    1, // FmulDoubleReal -> STATE_FPU
    1, // FimulWordInteger -> STATE_FPU
    1, // FimulDwordInteger -> STATE_FPU
    1, // FsubSt0Stj -> STATE_FPU
    1, // FsubrSt0Stj -> STATE_FPU
    1, // FsubStiSt0 -> STATE_FPU
    1, // FsubpStiSt0 -> STATE_FPU
    1, // FsubrStiSt0 -> STATE_FPU
    1, // FsubrpStiSt0 -> STATE_FPU
    1, // FsubSingleReal -> STATE_FPU
    1, // FsubrSingleReal -> STATE_FPU
    1, // FsubDoubleReal -> STATE_FPU
    1, // FsubrDoubleReal -> STATE_FPU
    1, // FisubWordInteger -> STATE_FPU
    1, // FisubrWordInteger -> STATE_FPU
    1, // FisubDwordInteger -> STATE_FPU
    1, // FisubrDwordInteger -> STATE_FPU
    1, // FdivSt0Stj -> STATE_FPU
    1, // FdivrSt0Stj -> STATE_FPU
    1, // FdivStiSt0 -> STATE_FPU
    1, // FdivpStiSt0 -> STATE_FPU
    1, // FdivrStiSt0 -> STATE_FPU
    1, // FdivrpStiSt0 -> STATE_FPU
    1, // FdivSingleReal -> STATE_FPU
    1, // FdivrSingleReal -> STATE_FPU
    1, // FdivDoubleReal -> STATE_FPU
    1, // FdivrDoubleReal -> STATE_FPU
    1, // FidivWordInteger -> STATE_FPU
    1, // FidivrWordInteger -> STATE_FPU
    1, // FidivDwordInteger -> STATE_FPU
    1, // FidivrDwordInteger -> STATE_FPU
    1, // FcomSti -> STATE_FPU
    1, // FcompSti -> STATE_FPU
    1, // FucomSti -> STATE_FPU
    1, // FucompSti -> STATE_FPU
    1, // FcomiSt0Stj -> STATE_FPU
    1, // FcomipSt0Stj -> STATE_FPU
    1, // FucomiSt0Stj -> STATE_FPU
    1, // FucomipSt0Stj -> STATE_FPU
    1, // FcomSingleReal -> STATE_FPU
    1, // FcompSingleReal -> STATE_FPU
    1, // FcomDoubleReal -> STATE_FPU
    1, // FcompDoubleReal -> STATE_FPU
    1, // FicomWordInteger -> STATE_FPU
    1, // FicompWordInteger -> STATE_FPU
    1, // FicomDwordInteger -> STATE_FPU
    1, // FicompDwordInteger -> STATE_FPU
    1, // FcmovbSt0Stj -> STATE_FPU
    1, // FcmoveSt0Stj -> STATE_FPU
    1, // FcmovbeSt0Stj -> STATE_FPU
    1, // FcmovuSt0Stj -> STATE_FPU
    1, // FcmovnbSt0Stj -> STATE_FPU
    1, // FcmovneSt0Stj -> STATE_FPU
    1, // FcmovnbeSt0Stj -> STATE_FPU
    1, // FcmovnuSt0Stj -> STATE_FPU
    1, // Fcompp -> STATE_FPU
    1, // Fucompp -> STATE_FPU
    1, // FxchSti -> STATE_FPU
    1, // Fnop -> STATE_FPU
    1, // Fplegacy -> STATE_FPU
    1, // Fchs -> STATE_FPU
    1, // Fabs -> STATE_FPU
    1, // Ftst -> STATE_FPU
    1, // Fxam -> STATE_FPU
    1, // Fdecstp -> STATE_FPU
    1, // Fincstp -> STATE_FPU
    1, // FfreeSti -> STATE_FPU
    1, // FfreepSti -> STATE_FPU
    1, // F2XM1 -> STATE_FPU
    1, // FYL2X -> STATE_FPU
    1, // Fptan -> STATE_FPU
    1, // Fpatan -> STATE_FPU
    1, // Fxtract -> STATE_FPU
    1, // FPREM1 -> STATE_FPU
    1, // Fprem -> STATE_FPU
    1, // FYL2XP1 -> STATE_FPU
    1, // Fsqrt -> STATE_FPU
    1, // Fsincos -> STATE_FPU
    1, // Frndint -> STATE_FPU
    1, // Fscale -> STATE_FPU
    1, // Fsin -> STATE_FPU
    1, // Fcos -> STATE_FPU
    1, // Fpuesc -> STATE_FPU
    0, // Cpuid -> STATE_NONE
    0, // BswapRx -> STATE_NONE
    0, // BswapErx -> STATE_NONE
    0, // Invd -> STATE_NONE
    0, // Wbinvd -> STATE_NONE
    0, // XaddEbGb -> STATE_NONE
    0, // XaddEwGw -> STATE_NONE
    0, // XaddEdGd -> STATE_NONE
    0, // CmpxchgEbGb -> STATE_NONE
    0, // CmpxchgEwGw -> STATE_NONE
    0, // CmpxchgEdGd -> STATE_NONE
    0, // Invlpg -> STATE_NONE
    0, // Cmpxchg8b -> STATE_NONE
    0, // Wrmsr -> STATE_NONE
    0, // Rdmsr -> STATE_NONE
    0, // Rdtsc -> STATE_NONE
    2, // PunpcklbwPqQd -> STATE_MMX
    2, // PunpcklwdPqQd -> STATE_MMX
    2, // PunpckldqPqQd -> STATE_MMX
    2, // PacksswbPqQq -> STATE_MMX
    2, // PcmpgtbPqQq -> STATE_MMX
    2, // PcmpgtwPqQq -> STATE_MMX
    2, // PcmpgtdPqQq -> STATE_MMX
    2, // PackuswbPqQq -> STATE_MMX
    2, // PunpckhbwPqQq -> STATE_MMX
    2, // PunpckhwdPqQq -> STATE_MMX
    2, // PunpckhdqPqQq -> STATE_MMX
    2, // PackssdwPqQq -> STATE_MMX
    2, // MovdPqEd -> STATE_MMX
    2, // MovqPqQq -> STATE_MMX
    2, // PcmpeqbPqQq -> STATE_MMX
    2, // PcmpeqwPqQq -> STATE_MMX
    2, // PcmpeqdPqQq -> STATE_MMX
    2, // Emms -> STATE_MMX
    2, // MovdEdPq -> STATE_MMX
    2, // MovqQqPq -> STATE_MMX
    2, // PsrlwPqQq -> STATE_MMX
    2, // PsrldPqQq -> STATE_MMX
    2, // PsrlqPqQq -> STATE_MMX
    2, // PmullwPqQq -> STATE_MMX
    2, // PsubusbPqQq -> STATE_MMX
    2, // PsubuswPqQq -> STATE_MMX
    2, // PandPqQq -> STATE_MMX
    2, // PaddusbPqQq -> STATE_MMX
    2, // PadduswPqQq -> STATE_MMX
    2, // PandnPqQq -> STATE_MMX
    2, // PsrawPqQq -> STATE_MMX
    2, // PsradPqQq -> STATE_MMX
    2, // PmulhwPqQq -> STATE_MMX
    2, // PsubsbPqQq -> STATE_MMX
    2, // PsubswPqQq -> STATE_MMX
    2, // PorPqQq -> STATE_MMX
    2, // PaddsbPqQq -> STATE_MMX
    2, // PaddswPqQq -> STATE_MMX
    2, // PxorPqQq -> STATE_MMX
    2, // PsllwPqQq -> STATE_MMX
    2, // PslldPqQq -> STATE_MMX
    2, // PsllqPqQq -> STATE_MMX
    2, // PmaddwdPqQq -> STATE_MMX
    2, // PsubbPqQq -> STATE_MMX
    2, // PsubwPqQq -> STATE_MMX
    2, // PsubdPqQq -> STATE_MMX
    2, // PaddbPqQq -> STATE_MMX
    2, // PaddwPqQq -> STATE_MMX
    2, // PadddPqQq -> STATE_MMX
    2, // PsrlwNqIb -> STATE_MMX
    2, // PsrawNqIb -> STATE_MMX
    2, // PsllwNqIb -> STATE_MMX
    2, // PsrldNqIb -> STATE_MMX
    2, // PsradNqIb -> STATE_MMX
    2, // PslldNqIb -> STATE_MMX
    2, // PsrlqNqIb -> STATE_MMX
    2, // PsllqNqIb -> STATE_MMX
    2, // MovqEqPq -> STATE_MMX
    2, // Femms -> STATE_MMX
    2, // Pf2idPqQq -> STATE_MMX
    2, // Pf2iwPqQq -> STATE_MMX
    2, // PfaccPqQq -> STATE_MMX
    2, // PfaddPqQq -> STATE_MMX
    2, // PfcmpeqPqQq -> STATE_MMX
    2, // PfcmpgePqQq -> STATE_MMX
    2, // PfcmpgtPqQq -> STATE_MMX
    2, // PfmaxPqQq -> STATE_MMX
    2, // PfminPqQq -> STATE_MMX
    2, // PfmulPqQq -> STATE_MMX
    2, // PfnaccPqQq -> STATE_MMX
    2, // PfpnaccPqQq -> STATE_MMX
    2, // PfrcpPqQq -> STATE_MMX
    2, // Pfrcpit1PqQq -> STATE_MMX
    2, // Pfrcpit2PqQq -> STATE_MMX
    2, // Pfrsqit1PqQq -> STATE_MMX
    2, // PfrsqrtPqQq -> STATE_MMX
    2, // PfsubPqQq -> STATE_MMX
    2, // PfsubrPqQq -> STATE_MMX
    2, // Pi2fdPqQq -> STATE_MMX
    2, // Pi2fwPqQq -> STATE_MMX
    2, // PmulhrwPqQq -> STATE_MMX
    2, // PswapdPqQq -> STATE_MMX
    0, // PrefetchwMb -> STATE_NONE
    0, // SyscallLegacy -> STATE_NONE
    0, // SysretLegacy -> STATE_NONE
    0, // CmovbGwEw -> STATE_NONE
    0, // CmovbeGwEw -> STATE_NONE
    0, // CmovlGwEw -> STATE_NONE
    0, // CmovleGwEw -> STATE_NONE
    0, // CmovnbGwEw -> STATE_NONE
    0, // CmovnbeGwEw -> STATE_NONE
    0, // CmovnlGwEw -> STATE_NONE
    0, // CmovnleGwEw -> STATE_NONE
    0, // CmovnoGwEw -> STATE_NONE
    0, // CmovnpGwEw -> STATE_NONE
    0, // CmovnsGwEw -> STATE_NONE
    0, // CmovnzGwEw -> STATE_NONE
    0, // CmovoGwEw -> STATE_NONE
    0, // CmovpGwEw -> STATE_NONE
    0, // CmovsGwEw -> STATE_NONE
    0, // CmovzGwEw -> STATE_NONE
    0, // CmovbGdEd -> STATE_NONE
    0, // CmovbeGdEd -> STATE_NONE
    0, // CmovlGdEd -> STATE_NONE
    0, // CmovleGdEd -> STATE_NONE
    0, // CmovnbGdEd -> STATE_NONE
    0, // CmovnbeGdEd -> STATE_NONE
    0, // CmovnlGdEd -> STATE_NONE
    0, // CmovnleGdEd -> STATE_NONE
    0, // CmovnoGdEd -> STATE_NONE
    0, // CmovnpGdEd -> STATE_NONE
    0, // CmovnsGdEd -> STATE_NONE
    0, // CmovnzGdEd -> STATE_NONE
    0, // CmovoGdEd -> STATE_NONE
    0, // CmovpGdEd -> STATE_NONE
    0, // CmovsGdEd -> STATE_NONE
    0, // CmovzGdEd -> STATE_NONE
    0, // Rdpmc -> STATE_NONE
    0, // Ud0 -> STATE_NONE
    0, // Ud1 -> STATE_NONE
    0, // Ud2 -> STATE_NONE
    0, // Fxsave -> STATE_NONE
    0, // Fxrstor -> STATE_NONE
    3, // Ldmxcsr -> STATE_SSE
    3, // Stmxcsr -> STATE_SSE
    0, // PrefetchMb -> STATE_NONE
    0, // Prefetcht0Mb -> STATE_NONE
    0, // Prefetcht1Mb -> STATE_NONE
    0, // Prefetcht2Mb -> STATE_NONE
    0, // PrefetchntaMb -> STATE_NONE
    3, // AndpsVpsWps -> STATE_SSE
    3, // OrpsVpsWps -> STATE_SSE
    3, // XorpsVpsWps -> STATE_SSE
    3, // AndnpsVpsWps -> STATE_SSE
    3, // MovupsVpsWps -> STATE_SSE
    3, // MovupsWpsVps -> STATE_SSE
    3, // MovssVssWss -> STATE_SSE
    3, // MovssWssVss -> STATE_SSE
    3, // MovlpsVpsMq -> STATE_SSE
    3, // MovhlpsVpsWps -> STATE_SSE
    3, // MovlpsMqVps -> STATE_SSE
    3, // MovhpsVpsMq -> STATE_SSE
    3, // MovlhpsVpsWps -> STATE_SSE
    3, // MovhpsMqVps -> STATE_SSE
    3, // MovapsVpsWps -> STATE_SSE
    3, // MovapsWpsVps -> STATE_SSE
    3, // MovntpsMpsVps -> STATE_SSE
    3, // Cvtpi2psVpsQq -> STATE_SSE
    3, // Cvtsi2ssVssEd -> STATE_SSE
    3, // Cvttps2piPqWps -> STATE_SSE
    3, // Cvtps2piPqWps -> STATE_SSE
    3, // Cvttss2siGdWss -> STATE_SSE
    3, // Cvtss2siGdWss -> STATE_SSE
    3, // UcomissVssWss -> STATE_SSE
    3, // ComissVssWss -> STATE_SSE
    3, // MovmskpsGdUps -> STATE_SSE
    3, // MovmskpdGdUpd -> STATE_SSE
    3, // RsqrtpsVpsWps -> STATE_SSE
    3, // RsqrtssVssWss -> STATE_SSE
    3, // RcppsVpsWps -> STATE_SSE
    3, // RcpssVssWss -> STATE_SSE
    2, // PshufwPqQqIb -> STATE_MMX
    3, // PshuflwVdqWdqIb -> STATE_SSE
    2, // PinsrwPqEwIb -> STATE_MMX
    2, // PextrwGdNqIb -> STATE_MMX
    3, // ShufpsVpsWpsIb -> STATE_SSE
    2, // PmovmskbGdNq -> STATE_MMX
    2, // PminubPqQq -> STATE_MMX
    2, // PmaxubPqQq -> STATE_MMX
    2, // PavgbPqQq -> STATE_MMX
    2, // PavgwPqQq -> STATE_MMX
    2, // PmulhuwPqQq -> STATE_MMX
    2, // MovntqMqPq -> STATE_MMX
    2, // PminswPqQq -> STATE_MMX
    2, // PmaxswPqQq -> STATE_MMX
    2, // PsadbwPqQq -> STATE_MMX
    2, // MaskmovqPqNq -> STATE_MMX
    3, // AddpsVpsWps -> STATE_SSE
    3, // AddpdVpdWpd -> STATE_SSE
    3, // AddssVssWss -> STATE_SSE
    3, // AddsdVsdWsd -> STATE_SSE
    3, // MulpsVpsWps -> STATE_SSE
    3, // MulpdVpdWpd -> STATE_SSE
    3, // MulssVssWss -> STATE_SSE
    3, // MulsdVsdWsd -> STATE_SSE
    3, // SubpsVpsWps -> STATE_SSE
    3, // SubpdVpdWpd -> STATE_SSE
    3, // SubssVssWss -> STATE_SSE
    3, // SubsdVsdWsd -> STATE_SSE
    3, // MinpsVpsWps -> STATE_SSE
    3, // MinpdVpdWpd -> STATE_SSE
    3, // MinssVssWss -> STATE_SSE
    3, // MinsdVsdWsd -> STATE_SSE
    3, // DivpsVpsWps -> STATE_SSE
    3, // DivpdVpdWpd -> STATE_SSE
    3, // DivssVssWss -> STATE_SSE
    3, // DivsdVsdWsd -> STATE_SSE
    3, // MaxpsVpsWps -> STATE_SSE
    3, // MaxpdVpdWpd -> STATE_SSE
    3, // MaxssVssWss -> STATE_SSE
    3, // MaxsdVsdWsd -> STATE_SSE
    3, // SqrtpsVpsWps -> STATE_SSE
    3, // SqrtpdVpdWpd -> STATE_SSE
    3, // SqrtssVssWss -> STATE_SSE
    3, // SqrtsdVsdWsd -> STATE_SSE
    3, // CmppsVpsWpsIb -> STATE_SSE
    3, // CmppdVpdWpdIb -> STATE_SSE
    3, // CmpssVssWssIb -> STATE_SSE
    3, // CmpsdVsdWsdIb -> STATE_SSE
    3, // Cvtps2pdVpdWps -> STATE_SSE
    3, // Cvtpd2psVpsWpd -> STATE_SSE
    3, // Cvtss2sdVsdWss -> STATE_SSE
    3, // Cvtsd2ssVssWsd -> STATE_SSE
    3, // MovsdVsdWsd -> STATE_SSE
    3, // MovsdWsdVsd -> STATE_SSE
    3, // Cvtpi2pdVpdQq -> STATE_SSE
    3, // Cvtsi2sdVsdEd -> STATE_SSE
    3, // Cvttpd2piPqWpd -> STATE_SSE
    3, // Cvttsd2siGdWsd -> STATE_SSE
    3, // Cvtpd2piPqWpd -> STATE_SSE
    3, // Cvtsd2siGdWsd -> STATE_SSE
    3, // UcomisdVsdWsd -> STATE_SSE
    3, // ComisdVsdWsd -> STATE_SSE
    3, // Cvtdq2psVpsWdq -> STATE_SSE
    3, // Cvtps2dqVdqWps -> STATE_SSE
    3, // Cvttps2dqVdqWps -> STATE_SSE
    3, // UnpckhpdVpdWdq -> STATE_SSE
    3, // UnpcklpdVpdWdq -> STATE_SSE
    3, // PunpckhdqVdqWdq -> STATE_SSE
    3, // PunpckldqVdqWdq -> STATE_SSE
    3, // MovapdVpdWpd -> STATE_SSE
    3, // MovapdWpdVpd -> STATE_SSE
    3, // MovdqaVdqWdq -> STATE_SSE
    3, // MovdqaWdqVdq -> STATE_SSE
    3, // MovdquVdqWdq -> STATE_SSE
    3, // MovdquWdqVdq -> STATE_SSE
    3, // MovhpdMqVsd -> STATE_SSE
    3, // MovhpdVsdMq -> STATE_SSE
    3, // MovlpdMqVsd -> STATE_SSE
    3, // MovlpdVsdMq -> STATE_SSE
    3, // MovntdqMdqVdq -> STATE_SSE
    3, // MovntpdMpdVpd -> STATE_SSE
    3, // MovupdVpdWpd -> STATE_SSE
    3, // MovupdWpdVpd -> STATE_SSE
    3, // AndnpdVpdWpd -> STATE_SSE
    3, // AndpdVpdWpd -> STATE_SSE
    3, // OrpdVpdWpd -> STATE_SSE
    3, // XorpdVpdWpd -> STATE_SSE
    3, // PandVdqWdq -> STATE_SSE
    3, // PandnVdqWdq -> STATE_SSE
    3, // PorVdqWdq -> STATE_SSE
    3, // PxorVdqWdq -> STATE_SSE
    3, // PunpcklbwVdqWdq -> STATE_SSE
    3, // PunpcklwdVdqWdq -> STATE_SSE
    3, // UnpcklpsVpsWdq -> STATE_SSE
    3, // UnpckhpsVpsWdq -> STATE_SSE
    3, // PackuswbVdqWdq -> STATE_SSE
    3, // PacksswbVdqWdq -> STATE_SSE
    3, // PcmpgtbVdqWdq -> STATE_SSE
    3, // PcmpgtwVdqWdq -> STATE_SSE
    3, // PcmpgtdVdqWdq -> STATE_SSE
    3, // PunpckhbwVdqWdq -> STATE_SSE
    3, // PunpckhwdVdqWdq -> STATE_SSE
    3, // PackssdwVdqWdq -> STATE_SSE
    3, // PunpcklqdqVdqWdq -> STATE_SSE
    3, // PunpckhqdqVdqWdq -> STATE_SSE
    3, // MovdVdqEd -> STATE_SSE
    3, // PshufdVdqWdqIb -> STATE_SSE
    3, // PshufhwVdqWdqIb -> STATE_SSE
    3, // PcmpeqbVdqWdq -> STATE_SSE
    3, // PcmpeqwVdqWdq -> STATE_SSE
    3, // PcmpeqdVdqWdq -> STATE_SSE
    3, // MovdEdVd -> STATE_SSE
    3, // MovqVqWq -> STATE_SSE
    0, // MovntiOp32MdGd -> STATE_NONE
    3, // PinsrwVdqEwIb -> STATE_SSE
    3, // PextrwGdUdqIb -> STATE_SSE
    3, // ShufpdVpdWpdIb -> STATE_SSE
    3, // PsrlwVdqWdq -> STATE_SSE
    3, // PsrldVdqWdq -> STATE_SSE
    3, // PsrlqVdqWdq -> STATE_SSE
    2, // PaddqPqQq -> STATE_MMX
    2, // PsubqPqQq -> STATE_MMX
    3, // PaddqVdqWdq -> STATE_SSE
    3, // PmullwVdqWdq -> STATE_SSE
    3, // MovqWqVq -> STATE_SSE
    3, // Movdq2qPqUdq -> STATE_SSE
    3, // Movq2dqVdqQq -> STATE_SSE
    3, // PmovmskbGdUdq -> STATE_SSE
    3, // PsubusbVdqWdq -> STATE_SSE
    3, // PsubuswVdqWdq -> STATE_SSE
    3, // PminubVdqWdq -> STATE_SSE
    3, // PaddusbVdqWdq -> STATE_SSE
    3, // PadduswVdqWdq -> STATE_SSE
    3, // PmaxubVdqWdq -> STATE_SSE
    3, // PavgbVdqWdq -> STATE_SSE
    3, // PsrawVdqWdq -> STATE_SSE
    3, // PsradVdqWdq -> STATE_SSE
    3, // PavgwVdqWdq -> STATE_SSE
    3, // PmulhuwVdqWdq -> STATE_SSE
    3, // PmulhwVdqWdq -> STATE_SSE
    3, // Cvttpd2dqVqWpd -> STATE_SSE
    3, // Cvtpd2dqVqWpd -> STATE_SSE
    3, // Cvtdq2pdVpdWq -> STATE_SSE
    3, // PsubsbVdqWdq -> STATE_SSE
    3, // PsubswVdqWdq -> STATE_SSE
    3, // PminswVdqWdq -> STATE_SSE
    3, // PmaxswVdqWdq -> STATE_SSE
    3, // PaddsbVdqWdq -> STATE_SSE
    3, // PaddswVdqWdq -> STATE_SSE
    3, // PsllwVdqWdq -> STATE_SSE
    3, // PslldVdqWdq -> STATE_SSE
    3, // PsllqVdqWdq -> STATE_SSE
    2, // PmuludqPqQq -> STATE_MMX
    3, // PmuludqVdqWdq -> STATE_SSE
    3, // PmaddwdVdqWdq -> STATE_SSE
    3, // PsadbwVdqWdq -> STATE_SSE
    3, // MaskmovdquVdqUdq -> STATE_SSE
    3, // PsubbVdqWdq -> STATE_SSE
    3, // PsubwVdqWdq -> STATE_SSE
    3, // PsubdVdqWdq -> STATE_SSE
    3, // PsubqVdqWdq -> STATE_SSE
    3, // PaddbVdqWdq -> STATE_SSE
    3, // PaddwVdqWdq -> STATE_SSE
    3, // PadddVdqWdq -> STATE_SSE
    3, // PsrlwUdqIb -> STATE_SSE
    3, // PsrawUdqIb -> STATE_SSE
    3, // PsllwUdqIb -> STATE_SSE
    3, // PsrldUdqIb -> STATE_SSE
    3, // PsradUdqIb -> STATE_SSE
    3, // PslldUdqIb -> STATE_SSE
    3, // PsrlqUdqIb -> STATE_SSE
    3, // PsllqUdqIb -> STATE_SSE
    3, // PsrldqUdqIb -> STATE_SSE
    3, // PslldqUdqIb -> STATE_SSE
    0, // Lfence -> STATE_NONE
    0, // Sfence -> STATE_NONE
    0, // Mfence -> STATE_NONE
    3, // MovddupVpdWq -> STATE_SSE
    3, // MovsldupVpsWps -> STATE_SSE
    3, // MovshdupVpsWps -> STATE_SSE
    3, // HaddpdVpdWpd -> STATE_SSE
    3, // HaddpsVpsWps -> STATE_SSE
    3, // HsubpdVpdWpd -> STATE_SSE
    3, // HsubpsVpsWps -> STATE_SSE
    3, // AddsubpdVpdWpd -> STATE_SSE
    3, // AddsubpsVpsWps -> STATE_SSE
    3, // LddquVdqMdq -> STATE_SSE
    2, // PshufbPqQq -> STATE_MMX
    2, // PhaddwPqQq -> STATE_MMX
    2, // PhadddPqQq -> STATE_MMX
    2, // PhaddswPqQq -> STATE_MMX
    2, // PmaddubswPqQq -> STATE_MMX
    2, // PhsubswPqQq -> STATE_MMX
    2, // PhsubwPqQq -> STATE_MMX
    2, // PhsubdPqQq -> STATE_MMX
    2, // PsignbPqQq -> STATE_MMX
    2, // PsignwPqQq -> STATE_MMX
    2, // PsigndPqQq -> STATE_MMX
    2, // PmulhrswPqQq -> STATE_MMX
    2, // PabsbPqQq -> STATE_MMX
    2, // PabswPqQq -> STATE_MMX
    2, // PabsdPqQq -> STATE_MMX
    2, // PalignrPqQqIb -> STATE_MMX
    3, // PshufbVdqWdq -> STATE_SSE
    3, // PhaddwVdqWdq -> STATE_SSE
    3, // PhadddVdqWdq -> STATE_SSE
    3, // PhaddswVdqWdq -> STATE_SSE
    3, // PmaddubswVdqWdq -> STATE_SSE
    3, // PhsubswVdqWdq -> STATE_SSE
    3, // PhsubwVdqWdq -> STATE_SSE
    3, // PhsubdVdqWdq -> STATE_SSE
    3, // PsignbVdqWdq -> STATE_SSE
    3, // PsignwVdqWdq -> STATE_SSE
    3, // PsigndVdqWdq -> STATE_SSE
    3, // PmulhrswVdqWdq -> STATE_SSE
    3, // PabsbVdqWdq -> STATE_SSE
    3, // PabswVdqWdq -> STATE_SSE
    3, // PabsdVdqWdq -> STATE_SSE
    3, // PalignrVdqWdqIb -> STATE_SSE
    3, // PblendvbVdqWdq -> STATE_SSE
    3, // BlendvpsVpsWps -> STATE_SSE
    3, // BlendvpdVpdWpd -> STATE_SSE
    3, // PmovsxbwVdqWq -> STATE_SSE
    3, // PmovsxbdVdqWd -> STATE_SSE
    3, // PmovsxbqVdqWw -> STATE_SSE
    3, // PmovsxwdVdqWq -> STATE_SSE
    3, // PmovsxwqVdqWd -> STATE_SSE
    3, // PmovsxdqVdqWq -> STATE_SSE
    3, // PmovzxbwVdqWq -> STATE_SSE
    3, // PmovzxbdVdqWd -> STATE_SSE
    3, // PmovzxbqVdqWw -> STATE_SSE
    3, // PmovzxwdVdqWq -> STATE_SSE
    3, // PmovzxwqVdqWd -> STATE_SSE
    3, // PmovzxdqVdqWq -> STATE_SSE
    3, // PtestVdqWdq -> STATE_SSE
    3, // PmuldqVdqWdq -> STATE_SSE
    3, // PcmpeqqVdqWdq -> STATE_SSE
    3, // PackusdwVdqWdq -> STATE_SSE
    3, // PminsbVdqWdq -> STATE_SSE
    3, // PminsdVdqWdq -> STATE_SSE
    3, // PminuwVdqWdq -> STATE_SSE
    3, // PminudVdqWdq -> STATE_SSE
    3, // PmaxsbVdqWdq -> STATE_SSE
    3, // PmaxsdVdqWdq -> STATE_SSE
    3, // PmaxuwVdqWdq -> STATE_SSE
    3, // PmaxudVdqWdq -> STATE_SSE
    3, // PmulldVdqWdq -> STATE_SSE
    3, // PhminposuwVdqWdq -> STATE_SSE
    3, // RoundpsVpsWpsIb -> STATE_SSE
    3, // RoundpdVpdWpdIb -> STATE_SSE
    3, // RoundssVssWssIb -> STATE_SSE
    3, // RoundsdVsdWsdIb -> STATE_SSE
    3, // BlendpsVpsWpsIb -> STATE_SSE
    3, // BlendpdVpdWpdIb -> STATE_SSE
    3, // PblendwVdqWdqIb -> STATE_SSE
    3, // PextrbEdVdqIbR -> STATE_SSE
    3, // PextrbMbVdqIbM -> STATE_SSE
    3, // PextrwEdVdqIbR -> STATE_SSE
    3, // PextrwMwVdqIbM -> STATE_SSE
    3, // PextrdEdVdqIb -> STATE_SSE
    3, // PextrqEqVdqIb -> STATE_SSE
    3, // ExtractpsEdVpsIb -> STATE_SSE
    3, // PinsrbVdqEbIb -> STATE_SSE
    3, // InsertpsVpsWssIb -> STATE_SSE
    3, // PinsrdVdqEdIb -> STATE_SSE
    3, // PinsrqVdqEqIb -> STATE_SSE
    3, // DppsVpsWpsIb -> STATE_SSE
    3, // DppdVpdWpdIb -> STATE_SSE
    3, // MpsadbwVdqWdqIb -> STATE_SSE
    3, // MovntdqaVdqMdq -> STATE_SSE
    0, // Crc32GdEb -> STATE_NONE
    0, // Crc32GdEw -> STATE_NONE
    0, // Crc32GdEd -> STATE_NONE
    0, // Crc32GdEq -> STATE_NONE
    3, // PcmpgtqVdqWdq -> STATE_SSE
    3, // PcmpestrmVdqWdqIb -> STATE_SSE
    3, // PcmpestriVdqWdqIb -> STATE_SSE
    3, // PcmpistrmVdqWdqIb -> STATE_SSE
    3, // PcmpistriVdqWdqIb -> STATE_SSE
    0, // MovbeGwMw -> STATE_NONE
    0, // MovbeGdMd -> STATE_NONE
    0, // MovbeGqMq -> STATE_NONE
    0, // MovbeMwGw -> STATE_NONE
    0, // MovbeMdGd -> STATE_NONE
    0, // MovbeMqGq -> STATE_NONE
    0, // PopcntGwEw -> STATE_NONE
    0, // PopcntGdEd -> STATE_NONE
    0, // PopcntGqEq -> STATE_NONE
    0, // Xrstor -> STATE_NONE
    0, // Xsave -> STATE_NONE
    0, // Xsavec -> STATE_NONE
    0, // Xsetbv -> STATE_NONE
    0, // Xgetbv -> STATE_NONE
    0, // Xsaveopt -> STATE_NONE
    0, // Xsaves -> STATE_NONE
    0, // Xrstors -> STATE_NONE
    3, // AesimcVdqWdq -> STATE_SSE
    3, // AeskeygenassistVdqWdqIb -> STATE_SSE
    3, // AesencVdqWdq -> STATE_SSE
    3, // AesenclastVdqWdq -> STATE_SSE
    3, // AesdecVdqWdq -> STATE_SSE
    3, // AesdeclastVdqWdq -> STATE_SSE
    3, // PclmulqdqVdqWdqIb -> STATE_SSE
    3, // Sha1nexteVdqWdq -> STATE_SSE
    3, // Sha1msg1VdqWdq -> STATE_SSE
    3, // Sha1msg2VdqWdq -> STATE_SSE
    3, // Sha256rnds2VdqWdq -> STATE_SSE
    3, // Sha256msg1VdqWdq -> STATE_SSE
    3, // Sha256msg2VdqWdq -> STATE_SSE
    3, // Sha1rnds4VdqWdqIb -> STATE_SSE
    3, // Gf2p8affineqbVdqWdqIb -> STATE_SSE
    3, // Gf2p8affineinvqbVdqWdqIb -> STATE_SSE
    3, // Gf2p8mulbVdqWdq -> STATE_SSE
    0, // LahfLm -> STATE_NONE
    0, // SahfLm -> STATE_NONE
    0, // Syscall -> STATE_NONE
    0, // Sysret -> STATE_NONE
    0, // XorEqGqZeroIdiom -> STATE_NONE
    0, // XorGqEqZeroIdiom -> STATE_NONE
    0, // SubEqGqZeroIdiom -> STATE_NONE
    0, // SubGqEqZeroIdiom -> STATE_NONE
    0, // AddGqEq -> STATE_NONE
    0, // OrGqEq -> STATE_NONE
    0, // AdcGqEq -> STATE_NONE
    0, // SbbGqEq -> STATE_NONE
    0, // AndGqEq -> STATE_NONE
    0, // SubGqEq -> STATE_NONE
    0, // XorGqEq -> STATE_NONE
    0, // CmpGqEq -> STATE_NONE
    0, // AddEqGq -> STATE_NONE
    0, // OrEqGq -> STATE_NONE
    0, // AdcEqGq -> STATE_NONE
    0, // SbbEqGq -> STATE_NONE
    0, // AndEqGq -> STATE_NONE
    0, // SubEqGq -> STATE_NONE
    0, // XorEqGq -> STATE_NONE
    0, // TestEqGq -> STATE_NONE
    0, // CmpEqGq -> STATE_NONE
    0, // AddRaxid -> STATE_NONE
    0, // OrRaxid -> STATE_NONE
    0, // AdcRaxid -> STATE_NONE
    0, // SbbRaxid -> STATE_NONE
    0, // AndRaxid -> STATE_NONE
    0, // SubRaxid -> STATE_NONE
    0, // XorRaxid -> STATE_NONE
    0, // TestRaxid -> STATE_NONE
    0, // CmpRaxid -> STATE_NONE
    0, // AddEqId -> STATE_NONE
    0, // OrEqId -> STATE_NONE
    0, // AdcEqId -> STATE_NONE
    0, // SbbEqId -> STATE_NONE
    0, // AndEqId -> STATE_NONE
    0, // SubEqId -> STATE_NONE
    0, // XorEqId -> STATE_NONE
    0, // TestEqId -> STATE_NONE
    0, // CmpEqId -> STATE_NONE
    0, // AddEqsIb -> STATE_NONE
    0, // OrEqsIb -> STATE_NONE
    0, // AdcEqsIb -> STATE_NONE
    0, // SbbEqsIb -> STATE_NONE
    0, // AndEqsIb -> STATE_NONE
    0, // SubEqsIb -> STATE_NONE
    0, // XorEqsIb -> STATE_NONE
    0, // TestEqsIb -> STATE_NONE
    0, // CmpEqsIb -> STATE_NONE
    0, // XchgEqGq -> STATE_NONE
    0, // XchgRrxRax -> STATE_NONE
    0, // LeaGqM -> STATE_NONE
    0, // MovOp64GdEd -> STATE_NONE
    0, // MovOp64EdGd -> STATE_NONE
    0, // MovGqEq -> STATE_NONE
    0, // MovEqGq -> STATE_NONE
    0, // MovEqId -> STATE_NONE
    0, // MovRaxoq -> STATE_NONE
    0, // MovOqRax -> STATE_NONE
    0, // MovEaxoq -> STATE_NONE
    0, // MovOqEax -> STATE_NONE
    0, // MovAxoq -> STATE_NONE
    0, // MovOqAx -> STATE_NONE
    0, // MovAloq -> STATE_NONE
    0, // MovOqAl -> STATE_NONE
    0, // RepMovsqYqXq -> STATE_NONE
    0, // RepCmpsqXqYq -> STATE_NONE
    0, // RepStosqYqRax -> STATE_NONE
    0, // RepLodsqRaxxq -> STATE_NONE
    0, // RepScasqRaxyq -> STATE_NONE
    0, // CallJq -> STATE_NONE
    0, // JmpJq -> STATE_NONE
    0, // JmpJbq -> STATE_NONE
    0, // JoJq -> STATE_NONE
    0, // JnoJq -> STATE_NONE
    0, // JbJq -> STATE_NONE
    0, // JnbJq -> STATE_NONE
    0, // JzJq -> STATE_NONE
    0, // JnzJq -> STATE_NONE
    0, // JbeJq -> STATE_NONE
    0, // JnbeJq -> STATE_NONE
    0, // JsJq -> STATE_NONE
    0, // JnsJq -> STATE_NONE
    0, // JpJq -> STATE_NONE
    0, // JnpJq -> STATE_NONE
    0, // JlJq -> STATE_NONE
    0, // JnlJq -> STATE_NONE
    0, // JleJq -> STATE_NONE
    0, // JnleJq -> STATE_NONE
    0, // JoJbq -> STATE_NONE
    0, // JnoJbq -> STATE_NONE
    0, // JbJbq -> STATE_NONE
    0, // JnbJbq -> STATE_NONE
    0, // JzJbq -> STATE_NONE
    0, // JnzJbq -> STATE_NONE
    0, // JbeJbq -> STATE_NONE
    0, // JnbeJbq -> STATE_NONE
    0, // JsJbq -> STATE_NONE
    0, // JnsJbq -> STATE_NONE
    0, // JpJbq -> STATE_NONE
    0, // JnpJbq -> STATE_NONE
    0, // JlJbq -> STATE_NONE
    0, // JnlJbq -> STATE_NONE
    0, // JleJbq -> STATE_NONE
    0, // JnleJbq -> STATE_NONE
    0, // EnterOp64IwIb -> STATE_NONE
    0, // LeaveOp64 -> STATE_NONE
    0, // IretOp64 -> STATE_NONE
    0, // ShldEqGq -> STATE_NONE
    0, // ShldEqGqIb -> STATE_NONE
    0, // ShrdEqGq -> STATE_NONE
    0, // ShrdEqGqIb -> STATE_NONE
    0, // ImulGqEq -> STATE_NONE
    0, // ImulGqEqId -> STATE_NONE
    0, // ImulGqEqsIb -> STATE_NONE
    0, // MovzxGqEb -> STATE_NONE
    0, // MovzxGqEw -> STATE_NONE
    0, // MovsxGqEb -> STATE_NONE
    0, // MovsxGqEw -> STATE_NONE
    0, // MovsxdGqEd -> STATE_NONE
    0, // BswapRrx -> STATE_NONE
    0, // BsfGqEq -> STATE_NONE
    0, // BsrGqEq -> STATE_NONE
    0, // BtEqGq -> STATE_NONE
    0, // BtsEqGq -> STATE_NONE
    0, // BtrEqGq -> STATE_NONE
    0, // BtcEqGq -> STATE_NONE
    0, // BtEqIb -> STATE_NONE
    0, // BtsEqIb -> STATE_NONE
    0, // BtrEqIb -> STATE_NONE
    0, // BtcEqIb -> STATE_NONE
    0, // NotEq -> STATE_NONE
    0, // NegEq -> STATE_NONE
    0, // RolEq -> STATE_NONE
    0, // RorEq -> STATE_NONE
    0, // RclEq -> STATE_NONE
    0, // RcrEq -> STATE_NONE
    0, // ShlEq -> STATE_NONE
    0, // ShrEq -> STATE_NONE
    0, // SarEq -> STATE_NONE
    0, // RolEqIb -> STATE_NONE
    0, // RorEqIb -> STATE_NONE
    0, // RclEqIb -> STATE_NONE
    0, // RcrEqIb -> STATE_NONE
    0, // ShlEqIb -> STATE_NONE
    0, // ShrEqIb -> STATE_NONE
    0, // SarEqIb -> STATE_NONE
    0, // RolEqI1 -> STATE_NONE
    0, // RorEqI1 -> STATE_NONE
    0, // RclEqI1 -> STATE_NONE
    0, // RcrEqI1 -> STATE_NONE
    0, // ShlEqI1 -> STATE_NONE
    0, // ShrEqI1 -> STATE_NONE
    0, // SarEqI1 -> STATE_NONE
    0, // MulRaxeq -> STATE_NONE
    0, // ImulRaxeq -> STATE_NONE
    0, // DivRaxeq -> STATE_NONE
    0, // IdivRaxeq -> STATE_NONE
    0, // IncEq -> STATE_NONE
    0, // DecEq -> STATE_NONE
    0, // CallEq -> STATE_NONE
    0, // CallfOp64Ep -> STATE_NONE
    0, // JmpEq -> STATE_NONE
    0, // JmpfOp64Ep -> STATE_NONE
    0, // PushfFq -> STATE_NONE
    0, // PopfFq -> STATE_NONE
    0, // CmpxchgEqGq -> STATE_NONE
    0, // Cdqe -> STATE_NONE
    0, // Cqo -> STATE_NONE
    0, // XaddEqGq -> STATE_NONE
    0, // RetOp64Iw -> STATE_NONE
    0, // RetOp64 -> STATE_NONE
    0, // RetfOp64Iw -> STATE_NONE
    0, // RetfOp64 -> STATE_NONE
    0, // CmovoGqEq -> STATE_NONE
    0, // CmovnoGqEq -> STATE_NONE
    0, // CmovbGqEq -> STATE_NONE
    0, // CmovnbGqEq -> STATE_NONE
    0, // CmovzGqEq -> STATE_NONE
    0, // CmovnzGqEq -> STATE_NONE
    0, // CmovbeGqEq -> STATE_NONE
    0, // CmovnbeGqEq -> STATE_NONE
    0, // CmovsGqEq -> STATE_NONE
    0, // CmovnsGqEq -> STATE_NONE
    0, // CmovpGqEq -> STATE_NONE
    0, // CmovnpGqEq -> STATE_NONE
    0, // CmovlGqEq -> STATE_NONE
    0, // CmovnlGqEq -> STATE_NONE
    0, // CmovleGqEq -> STATE_NONE
    0, // CmovnleGqEq -> STATE_NONE
    0, // PushEq -> STATE_NONE
    0, // PopEq -> STATE_NONE
    0, // PushOp64Id -> STATE_NONE
    0, // PushOp64SIb -> STATE_NONE
    0, // PushOp64Sw -> STATE_NONE
    0, // PopOp64Sw -> STATE_NONE
    0, // SgdtOp64Ms -> STATE_NONE
    0, // SidtOp64Ms -> STATE_NONE
    0, // LgdtOp64Ms -> STATE_NONE
    0, // LidtOp64Ms -> STATE_NONE
    0, // MovRrxiq -> STATE_NONE
    0, // LssGqMp -> STATE_NONE
    0, // LfsGqMp -> STATE_NONE
    0, // LgsGqMp -> STATE_NONE
    0, // CMPXCHG16B -> STATE_NONE
    0, // LoopneJbq -> STATE_NONE
    0, // LoopeJbq -> STATE_NONE
    0, // LoopJbq -> STATE_NONE
    0, // JrcxzJbq -> STATE_NONE
    3, // MovqEqVq -> STATE_SSE
    2, // MovqPqEq -> STATE_MMX
    3, // MovqVdqEq -> STATE_SSE
    3, // Cvtsi2ssVssEq -> STATE_SSE
    3, // Cvtsi2sdVsdEq -> STATE_SSE
    3, // Cvttss2siGqWss -> STATE_SSE
    3, // Cvttsd2siGqWsd -> STATE_SSE
    3, // Cvtss2siGqWss -> STATE_SSE
    3, // Cvtsd2siGqWsd -> STATE_SSE
    0, // MovntiOp64MdGd -> STATE_NONE
    0, // MovntiMqGq -> STATE_NONE
    0, // MovCr0rq -> STATE_NONE
    0, // MovCr2rq -> STATE_NONE
    0, // MovCr3rq -> STATE_NONE
    0, // MovCr4rq -> STATE_NONE
    0, // MovRqCr0 -> STATE_NONE
    0, // MovRqCr2 -> STATE_NONE
    0, // MovRqCr3 -> STATE_NONE
    0, // MovRqCr4 -> STATE_NONE
    0, // MovDqRq -> STATE_NONE
    0, // MovRqDq -> STATE_NONE
    0, // Swapgs -> STATE_NONE
    0, // RdfsbaseEd -> STATE_NONE
    0, // RdgsbaseEd -> STATE_NONE
    0, // RdfsbaseEq -> STATE_NONE
    0, // RdgsbaseEq -> STATE_NONE
    0, // WrfsbaseEd -> STATE_NONE
    0, // WrgsbaseEd -> STATE_NONE
    0, // WrfsbaseEq -> STATE_NONE
    0, // WrgsbaseEq -> STATE_NONE
    0, // Rdtscp -> STATE_NONE
    0, // VmxonMq -> STATE_NONE
    0, // Vmxoff -> STATE_NONE
    0, // Vmcall -> STATE_NONE
    0, // Vmlaunch -> STATE_NONE
    0, // Vmresume -> STATE_NONE
    0, // VmclearMq -> STATE_NONE
    0, // VmptrldMq -> STATE_NONE
    0, // VmptrstMq -> STATE_NONE
    0, // VmreadEdGd -> STATE_NONE
    0, // VmwriteGdEd -> STATE_NONE
    0, // VmreadEqGq -> STATE_NONE
    0, // VmwriteGqEq -> STATE_NONE
    0, // Invept -> STATE_NONE
    0, // Invvpid -> STATE_NONE
    0, // Vmfunc -> STATE_NONE
    0, // Getsec -> STATE_NONE
    0, // Vmrun -> STATE_NONE
    0, // Vmmcall -> STATE_NONE
    0, // Vmload -> STATE_NONE
    0, // Vmsave -> STATE_NONE
    0, // Stgi -> STATE_NONE
    0, // Clgi -> STATE_NONE
    0, // Skinit -> STATE_NONE
    0, // Invlpga -> STATE_NONE
    0, // Incsspd -> STATE_NONE
    0, // Incsspq -> STATE_NONE
    0, // Rdsspd -> STATE_NONE
    0, // Rdsspq -> STATE_NONE
    0, // Saveprevssp -> STATE_NONE
    0, // Rstorssp -> STATE_NONE
    0, // Wrssd -> STATE_NONE
    0, // Wrussd -> STATE_NONE
    0, // Wrssq -> STATE_NONE
    0, // Wrussq -> STATE_NONE
    0, // Setssbsy -> STATE_NONE
    0, // Clrssbsy -> STATE_NONE
    0, // Endbranch32 -> STATE_NONE
    0, // Endbranch64 -> STATE_NONE
    0, // Invpcid -> STATE_NONE
    0, // Rdpkru -> STATE_NONE
    0, // Wrpkru -> STATE_NONE
    0, // Clui -> STATE_NONE
    0, // Stui -> STATE_NONE
    0, // Testui -> STATE_NONE
    0, // Uiret -> STATE_NONE
    0, // SenduipiEq -> STATE_NONE
    0, // RdpidEd -> STATE_NONE
    0, // Serialize -> STATE_NONE
    0, // Wrmsrns -> STATE_NONE
    0, // Rdmsrlist -> STATE_NONE
    0, // Wrmsrlist -> STATE_NONE
    4, // Vzeroupper -> STATE_AVX
    4, // Vzeroall -> STATE_AVX
    4, // Vldmxcsr -> STATE_AVX
    4, // Vstmxcsr -> STATE_AVX
    4, // VmovapsVpsWps -> STATE_AVX
    4, // V128VmovapsWpsVps -> STATE_AVX
    4, // V256VmovapsWpsVps -> STATE_AVX
    4, // VmovapdVpdWpd -> STATE_AVX
    4, // V128VmovapdWpdVpd -> STATE_AVX
    4, // V256VmovapdWpdVpd -> STATE_AVX
    4, // VmovupsVpsWps -> STATE_AVX
    4, // V128VmovupsWpsVps -> STATE_AVX
    4, // V256VmovupsWpsVps -> STATE_AVX
    4, // VmovupdVpdWpd -> STATE_AVX
    4, // V128VmovupdWpdVpd -> STATE_AVX
    4, // V256VmovupdWpdVpd -> STATE_AVX
    4, // VmovdqaVdqWdq -> STATE_AVX
    4, // V128VmovdqaWdqVdq -> STATE_AVX
    4, // V256VmovdqaWdqVdq -> STATE_AVX
    4, // VmovdquVdqWdq -> STATE_AVX
    4, // V128VmovdquWdqVdq -> STATE_AVX
    4, // V256VmovdquWdqVdq -> STATE_AVX
    4, // V128VmovsdVsdHpdWsd -> STATE_AVX
    4, // V128VmovssVssHpsWss -> STATE_AVX
    4, // V128VmovsdWsdHpdVsd -> STATE_AVX
    4, // V128VmovssWssHpsVss -> STATE_AVX
    4, // V128VmovsdVsdWsd -> STATE_AVX
    4, // V128VmovssVssWss -> STATE_AVX
    4, // V128VmovsdWsdVsd -> STATE_AVX
    4, // V128VmovssWssVss -> STATE_AVX
    4, // V128VmovlpsVpsHpsMq -> STATE_AVX
    4, // V128VmovhlpsVpsHpsWps -> STATE_AVX
    4, // V128VmovhpsVpsHpsMq -> STATE_AVX
    4, // V128VmovlhpsVpsHpsWps -> STATE_AVX
    4, // V128VmovlpsMqVps -> STATE_AVX
    4, // V128VmovhpsMqVps -> STATE_AVX
    4, // V128VmovlpdMqVsd -> STATE_AVX
    4, // V128VmovhpdMqVsd -> STATE_AVX
    4, // V128VmovlpdVpdHpdMq -> STATE_AVX
    4, // V128VmovhpdVpdHpdMq -> STATE_AVX
    4, // V128VmovddupVpdWpd -> STATE_AVX
    4, // V256VmovddupVpdWpd -> STATE_AVX
    4, // VmovsldupVpsWps -> STATE_AVX
    4, // VmovshdupVpsWps -> STATE_AVX
    4, // VlddquVdqMdq -> STATE_AVX
    4, // V128VmovntdqaVdqMdq -> STATE_AVX
    4, // V256VmovntdqaVdqMdq -> STATE_AVX
    4, // V128VmovntpsMpsVps -> STATE_AVX
    4, // V256VmovntpsMpsVps -> STATE_AVX
    4, // V128VmovntpdMpdVpd -> STATE_AVX
    4, // V256VmovntpdMpdVpd -> STATE_AVX
    4, // V128VmovntdqMdqVdq -> STATE_AVX
    4, // V256VmovntdqMdqVdq -> STATE_AVX
    4, // VucomissVssWss -> STATE_AVX
    4, // VcomissVssWss -> STATE_AVX
    4, // VucomisdVsdWsd -> STATE_AVX
    4, // VcomisdVsdWsd -> STATE_AVX
    4, // VrsqrtssVssHpsWss -> STATE_AVX
    4, // VrsqrtpsVpsWps -> STATE_AVX
    4, // VrcpssVssHpsWss -> STATE_AVX
    4, // VrcppsVpsWps -> STATE_AVX
    4, // VandpsVpsHpsWps -> STATE_AVX
    4, // VandpdVpdHpdWpd -> STATE_AVX
    4, // VandnpsVpsHpsWps -> STATE_AVX
    4, // VandnpdVpdHpdWpd -> STATE_AVX
    4, // VorpsVpsHpsWps -> STATE_AVX
    4, // VorpdVpdHpdWpd -> STATE_AVX
    4, // VxorpsVpsHpsWps -> STATE_AVX
    4, // VxorpdVpdHpdWpd -> STATE_AVX
    4, // V128VpshufdVdqWdqIb -> STATE_AVX
    4, // V256VpshufdVdqWdqIb -> STATE_AVX
    4, // V128VpshufhwVdqWdqIb -> STATE_AVX
    4, // V256VpshufhwVdqWdqIb -> STATE_AVX
    4, // V128VpshuflwVdqWdqIb -> STATE_AVX
    4, // V256VpshuflwVdqWdqIb -> STATE_AVX
    4, // VhaddpdVpdHpdWpd -> STATE_AVX
    4, // VhaddpsVpsHpsWps -> STATE_AVX
    4, // VhsubpdVpdHpdWpd -> STATE_AVX
    4, // VhsubpsVpsHpsWps -> STATE_AVX
    4, // VshufpsVpsHpsWpsIb -> STATE_AVX
    4, // VshufpdVpdHpdWpdIb -> STATE_AVX
    4, // VaddsubpdVpdHpdWpd -> STATE_AVX
    4, // VaddsubpsVpsHpsWps -> STATE_AVX
    4, // VroundpsVpsWpsIb -> STATE_AVX
    4, // VroundpdVpdWpdIb -> STATE_AVX
    4, // VroundsdVsdHpdWsdIb -> STATE_AVX
    4, // VroundssVssHpsWssIb -> STATE_AVX
    4, // VdppsVpsHpsWpsIb -> STATE_AVX
    4, // VdppdVpdHpdWpdIb -> STATE_AVX
    4, // VaddpsVpsHpsWps -> STATE_AVX
    4, // VaddpdVpdHpdWpd -> STATE_AVX
    4, // VaddssVssHpsWss -> STATE_AVX
    4, // VaddsdVsdHpdWsd -> STATE_AVX
    4, // VmulpsVpsHpsWps -> STATE_AVX
    4, // VmulpdVpdHpdWpd -> STATE_AVX
    4, // VmulssVssHpsWss -> STATE_AVX
    4, // VmulsdVsdHpdWsd -> STATE_AVX
    4, // VsubpsVpsHpsWps -> STATE_AVX
    4, // VsubpdVpdHpdWpd -> STATE_AVX
    4, // VsubssVssHpsWss -> STATE_AVX
    4, // VsubsdVsdHpdWsd -> STATE_AVX
    4, // VdivpsVpsHpsWps -> STATE_AVX
    4, // VdivpdVpdHpdWpd -> STATE_AVX
    4, // VdivssVssHpsWss -> STATE_AVX
    4, // VdivsdVsdHpdWsd -> STATE_AVX
    4, // VmaxpsVpsHpsWps -> STATE_AVX
    4, // VmaxpdVpdHpdWpd -> STATE_AVX
    4, // VmaxssVssHpsWss -> STATE_AVX
    4, // VmaxsdVsdHpdWsd -> STATE_AVX
    4, // VminpsVpsHpsWps -> STATE_AVX
    4, // VminpdVpdHpdWpd -> STATE_AVX
    4, // VminssVssHpsWss -> STATE_AVX
    4, // VminsdVsdHpdWsd -> STATE_AVX
    4, // VsqrtpsVpsWps -> STATE_AVX
    4, // VsqrtpdVpdWpd -> STATE_AVX
    4, // VsqrtssVssHpsWss -> STATE_AVX
    4, // VsqrtsdVsdHpdWsd -> STATE_AVX
    4, // VcmppsVpsHpsWpsIb -> STATE_AVX
    4, // VcmppdVpdHpdWpdIb -> STATE_AVX
    4, // VcmpssVssHpsWssIb -> STATE_AVX
    4, // VcmpsdVsdHpdWsdIb -> STATE_AVX
    4, // V128VpsrlwVdqHdqWdq -> STATE_AVX
    4, // V256VpsrlwVdqHdqWdq -> STATE_AVX
    4, // V128VpsrldVdqHdqWdq -> STATE_AVX
    4, // V256VpsrldVdqHdqWdq -> STATE_AVX
    4, // V128VpsrlqVdqHdqWdq -> STATE_AVX
    4, // V256VpsrlqVdqHdqWdq -> STATE_AVX
    4, // V128VpsrawVdqHdqWdq -> STATE_AVX
    4, // V256VpsrawVdqHdqWdq -> STATE_AVX
    4, // V128VpsradVdqHdqWdq -> STATE_AVX
    4, // V256VpsradVdqHdqWdq -> STATE_AVX
    4, // V128VpsllwVdqHdqWdq -> STATE_AVX
    4, // V256VpsllwVdqHdqWdq -> STATE_AVX
    4, // V128VpslldVdqHdqWdq -> STATE_AVX
    4, // V256VpslldVdqHdqWdq -> STATE_AVX
    4, // V128VpsllqVdqHdqWdq -> STATE_AVX
    4, // V256VpsllqVdqHdqWdq -> STATE_AVX
    4, // V128VpsrlwUdqIb -> STATE_AVX
    4, // V256VpsrlwUdqIb -> STATE_AVX
    4, // V128VpsrawUdqIb -> STATE_AVX
    4, // V256VpsrawUdqIb -> STATE_AVX
    4, // V128VpsllwUdqIb -> STATE_AVX
    4, // V256VpsllwUdqIb -> STATE_AVX
    4, // V128VpsrldUdqIb -> STATE_AVX
    4, // V256VpsrldUdqIb -> STATE_AVX
    4, // V128VpsradUdqIb -> STATE_AVX
    4, // V256VpsradUdqIb -> STATE_AVX
    4, // V128VpslldUdqIb -> STATE_AVX
    4, // V256VpslldUdqIb -> STATE_AVX
    4, // V128VpsrlqUdqIb -> STATE_AVX
    4, // V256VpsrlqUdqIb -> STATE_AVX
    4, // V128VpsllqUdqIb -> STATE_AVX
    4, // V256VpsllqUdqIb -> STATE_AVX
    4, // V128VpsrldqUdqIb -> STATE_AVX
    4, // V256VpsrldqUdqIb -> STATE_AVX
    4, // V128VpslldqUdqIb -> STATE_AVX
    4, // V256VpslldqUdqIb -> STATE_AVX
    4, // V128VpmovmskbGdUdq -> STATE_AVX
    4, // V256VpmovmskbGdUdq -> STATE_AVX
    4, // VmovmskpsGdUps -> STATE_AVX
    4, // VmovmskpdGdUpd -> STATE_AVX
    4, // VunpcklpdVpdHpdWpd -> STATE_AVX
    4, // VunpckhpdVpdHpdWpd -> STATE_AVX
    4, // VunpcklpsVpsHpsWps -> STATE_AVX
    4, // VunpckhpsVpsHpsWps -> STATE_AVX
    4, // V128VpunpckhdqVdqHdqWdq -> STATE_AVX
    4, // V256VpunpckhdqVdqHdqWdq -> STATE_AVX
    4, // V128VpunpckldqVdqHdqWdq -> STATE_AVX
    4, // V256VpunpckldqVdqHdqWdq -> STATE_AVX
    4, // V128VpunpcklbwVdqHdqWdq -> STATE_AVX
    4, // V256VpunpcklbwVdqHdqWdq -> STATE_AVX
    4, // V128VpunpcklwdVdqHdqWdq -> STATE_AVX
    4, // V256VpunpcklwdVdqHdqWdq -> STATE_AVX
    4, // V128VpunpckhbwVdqHdqWdq -> STATE_AVX
    4, // V256VpunpckhbwVdqHdqWdq -> STATE_AVX
    4, // V128VpunpckhwdVdqHdqWdq -> STATE_AVX
    4, // V256VpunpckhwdVdqHdqWdq -> STATE_AVX
    4, // V128VpunpcklqdqVdqHdqWdq -> STATE_AVX
    4, // V256VpunpcklqdqVdqHdqWdq -> STATE_AVX
    4, // V128VpunpckhqdqVdqHdqWdq -> STATE_AVX
    4, // V256VpunpckhqdqVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpeqbVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpeqbVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpeqwVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpeqwVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpeqdVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpeqdVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpeqqVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpeqqVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpgtbVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpgtbVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpgtwVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpgtwVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpgtdVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpgtdVdqHdqWdq -> STATE_AVX
    4, // V128VpcmpgtqVdqHdqWdq -> STATE_AVX
    4, // V256VpcmpgtqVdqHdqWdq -> STATE_AVX
    4, // V128VpsubsbVdqHdqWdq -> STATE_AVX
    4, // V256VpsubsbVdqHdqWdq -> STATE_AVX
    4, // V128VpsubswVdqHdqWdq -> STATE_AVX
    4, // V256VpsubswVdqHdqWdq -> STATE_AVX
    4, // V128VpaddsbVdqHdqWdq -> STATE_AVX
    4, // V256VpaddsbVdqHdqWdq -> STATE_AVX
    4, // V128VpaddswVdqHdqWdq -> STATE_AVX
    4, // V256VpaddswVdqHdqWdq -> STATE_AVX
    4, // V128VpsubusbVdqHdqWdq -> STATE_AVX
    4, // V256VpsubusbVdqHdqWdq -> STATE_AVX
    4, // V128VpsubuswVdqHdqWdq -> STATE_AVX
    4, // V256VpsubuswVdqHdqWdq -> STATE_AVX
    4, // V128VpaddusbVdqHdqWdq -> STATE_AVX
    4, // V256VpaddusbVdqHdqWdq -> STATE_AVX
    4, // V128VpadduswVdqHdqWdq -> STATE_AVX
    4, // V256VpadduswVdqHdqWdq -> STATE_AVX
    4, // V128VpavgbVdqWdq -> STATE_AVX
    4, // V256VpavgbVdqWdq -> STATE_AVX
    4, // V128VpavgwVdqWdq -> STATE_AVX
    4, // V256VpavgwVdqWdq -> STATE_AVX
    4, // V128VpandnVdqHdqWdq -> STATE_AVX
    4, // V256VpandnVdqHdqWdq -> STATE_AVX
    4, // V128VpandVdqHdqWdq -> STATE_AVX
    4, // V256VpandVdqHdqWdq -> STATE_AVX
    4, // V128VporVdqHdqWdq -> STATE_AVX
    4, // V256VporVdqHdqWdq -> STATE_AVX
    4, // V128VpxorVdqHdqWdq -> STATE_AVX
    4, // V256VpxorVdqHdqWdq -> STATE_AVX
    4, // V128VpmulhrswVdqHdqWdq -> STATE_AVX
    4, // V256VpmulhrswVdqHdqWdq -> STATE_AVX
    4, // V128VpmuldqVdqHdqWdq -> STATE_AVX
    4, // V256VpmuldqVdqHdqWdq -> STATE_AVX
    4, // V128VpmuludqVdqHdqWdq -> STATE_AVX
    4, // V256VpmuludqVdqHdqWdq -> STATE_AVX
    4, // V128VpmulldVdqHdqWdq -> STATE_AVX
    4, // V256VpmulldVdqHdqWdq -> STATE_AVX
    4, // V128VpmullwVdqHdqWdq -> STATE_AVX
    4, // V256VpmullwVdqHdqWdq -> STATE_AVX
    4, // V128VpmulhwVdqHdqWdq -> STATE_AVX
    4, // V256VpmulhwVdqHdqWdq -> STATE_AVX
    4, // V128VpmulhuwVdqHdqWdq -> STATE_AVX
    4, // V256VpmulhuwVdqHdqWdq -> STATE_AVX
    4, // V128VpsadbwVdqHdqWdq -> STATE_AVX
    4, // V256VpsadbwVdqHdqWdq -> STATE_AVX
    4, // V128VmaskmovdquVdqUdq -> STATE_AVX
    4, // V128VpsubbVdqHdqWdq -> STATE_AVX
    4, // V256VpsubbVdqHdqWdq -> STATE_AVX
    4, // V128VpsubwVdqHdqWdq -> STATE_AVX
    4, // V256VpsubwVdqHdqWdq -> STATE_AVX
    4, // V128VpsubdVdqHdqWdq -> STATE_AVX
    4, // V256VpsubdVdqHdqWdq -> STATE_AVX
    4, // V128VpsubqVdqHdqWdq -> STATE_AVX
    4, // V256VpsubqVdqHdqWdq -> STATE_AVX
    4, // V128VpaddbVdqHdqWdq -> STATE_AVX
    4, // V256VpaddbVdqHdqWdq -> STATE_AVX
    4, // V128VpaddwVdqHdqWdq -> STATE_AVX
    4, // V256VpaddwVdqHdqWdq -> STATE_AVX
    4, // V128VpadddVdqHdqWdq -> STATE_AVX
    4, // V256VpadddVdqHdqWdq -> STATE_AVX
    4, // V128VpaddqVdqHdqWdq -> STATE_AVX
    4, // V256VpaddqVdqHdqWdq -> STATE_AVX
    4, // V128VpshufbVdqHdqWdq -> STATE_AVX
    4, // V256VpshufbVdqHdqWdq -> STATE_AVX
    4, // V128VphaddwVdqHdqWdq -> STATE_AVX
    4, // V256VphaddwVdqHdqWdq -> STATE_AVX
    4, // V128VphadddVdqHdqWdq -> STATE_AVX
    4, // V256VphadddVdqHdqWdq -> STATE_AVX
    4, // V128VphsubwVdqHdqWdq -> STATE_AVX
    4, // V256VphsubwVdqHdqWdq -> STATE_AVX
    4, // V128VphsubdVdqHdqWdq -> STATE_AVX
    4, // V256VphsubdVdqHdqWdq -> STATE_AVX
    4, // V128VphaddswVdqHdqWdq -> STATE_AVX
    4, // V256VphaddswVdqHdqWdq -> STATE_AVX
    4, // V128VphsubswVdqHdqWdq -> STATE_AVX
    4, // V256VphsubswVdqHdqWdq -> STATE_AVX
    4, // V128VpmaddwdVdqHdqWdq -> STATE_AVX
    4, // V256VpmaddwdVdqHdqWdq -> STATE_AVX
    4, // V128VpmaddubswVdqHdqWdq -> STATE_AVX
    4, // V256VpmaddubswVdqHdqWdq -> STATE_AVX
    4, // V128VpsignbVdqHdqWdq -> STATE_AVX
    4, // V256VpsignbVdqHdqWdq -> STATE_AVX
    4, // V128VpsignwVdqHdqWdq -> STATE_AVX
    4, // V256VpsignwVdqHdqWdq -> STATE_AVX
    4, // V128VpsigndVdqHdqWdq -> STATE_AVX
    4, // V256VpsigndVdqHdqWdq -> STATE_AVX
    4, // VtestpsVpsWps -> STATE_AVX
    4, // VtestpdVpdWpd -> STATE_AVX
    4, // VptestVdqWdq -> STATE_AVX
    4, // VbroadcastssVpsMss -> STATE_AVX
    4, // V256VbroadcastsdVpdMsd -> STATE_AVX
    4, // V256Vbroadcastf128VdqMdq -> STATE_AVX
    4, // V128VpabsbVdqWdq -> STATE_AVX
    4, // V256VpabsbVdqWdq -> STATE_AVX
    4, // V128VpabswVdqWdq -> STATE_AVX
    4, // V256VpabswVdqWdq -> STATE_AVX
    4, // V128VpabsdVdqWdq -> STATE_AVX
    4, // V256VpabsdVdqWdq -> STATE_AVX
    4, // V128VpacksswbVdqHdqWdq -> STATE_AVX
    4, // V256VpacksswbVdqHdqWdq -> STATE_AVX
    4, // V128VpackuswbVdqHdqWdq -> STATE_AVX
    4, // V256VpackuswbVdqHdqWdq -> STATE_AVX
    4, // V128VpackusdwVdqHdqWdq -> STATE_AVX
    4, // V256VpackusdwVdqHdqWdq -> STATE_AVX
    4, // V128VpackssdwVdqHdqWdq -> STATE_AVX
    4, // V256VpackssdwVdqHdqWdq -> STATE_AVX
    4, // VmaskmovpsVpsHpsMps -> STATE_AVX
    4, // VmaskmovpdVpdHpdMpd -> STATE_AVX
    4, // VmaskmovpsMpsHpsVps -> STATE_AVX
    4, // VmaskmovpdMpdHpdVpd -> STATE_AVX
    4, // V128VpmovsxbwVdqWq -> STATE_AVX
    4, // V128VpmovsxbdVdqWd -> STATE_AVX
    4, // V128VpmovsxbqVdqWw -> STATE_AVX
    4, // V128VpmovsxwdVdqWq -> STATE_AVX
    4, // V128VpmovsxwqVdqWd -> STATE_AVX
    4, // V128VpmovsxdqVdqWq -> STATE_AVX
    4, // V128VpmovzxbwVdqWq -> STATE_AVX
    4, // V128VpmovzxbdVdqWd -> STATE_AVX
    4, // V128VpmovzxbqVdqWw -> STATE_AVX
    4, // V128VpmovzxwdVdqWq -> STATE_AVX
    4, // V128VpmovzxwqVdqWd -> STATE_AVX
    4, // V128VpmovzxdqVdqWq -> STATE_AVX
    4, // V128VpminsbVdqHdqWdq -> STATE_AVX
    4, // V256VpminsbVdqHdqWdq -> STATE_AVX
    4, // V128VpminswVdqHdqWdq -> STATE_AVX
    4, // V256VpminswVdqHdqWdq -> STATE_AVX
    4, // V128VpminsdVdqHdqWdq -> STATE_AVX
    4, // V256VpminsdVdqHdqWdq -> STATE_AVX
    4, // V128VpminubVdqHdqWdq -> STATE_AVX
    4, // V256VpminubVdqHdqWdq -> STATE_AVX
    4, // V128VpminuwVdqHdqWdq -> STATE_AVX
    4, // V256VpminuwVdqHdqWdq -> STATE_AVX
    4, // V128VpminudVdqHdqWdq -> STATE_AVX
    4, // V256VpminudVdqHdqWdq -> STATE_AVX
    4, // V128VpmaxsbVdqHdqWdq -> STATE_AVX
    4, // V256VpmaxsbVdqHdqWdq -> STATE_AVX
    4, // V128VpmaxswVdqHdqWdq -> STATE_AVX
    4, // V256VpmaxswVdqHdqWdq -> STATE_AVX
    4, // V128VpmaxsdVdqHdqWdq -> STATE_AVX
    4, // V256VpmaxsdVdqHdqWdq -> STATE_AVX
    4, // V128VpmaxubVdqHdqWdq -> STATE_AVX
    4, // V256VpmaxubVdqHdqWdq -> STATE_AVX
    4, // V128VpmaxuwVdqHdqWdq -> STATE_AVX
    4, // V256VpmaxuwVdqHdqWdq -> STATE_AVX
    4, // V128VpmaxudVdqHdqWdq -> STATE_AVX
    4, // V256VpmaxudVdqHdqWdq -> STATE_AVX
    4, // V128VphminposuwVdqWdq -> STATE_AVX
    4, // VpermilpsVpsHpsWps -> STATE_AVX
    4, // VpermilpdVpdHpdWpd -> STATE_AVX
    4, // VpermilpsVpsWpsIb -> STATE_AVX
    4, // VpermilpdVpdWpdIb -> STATE_AVX
    4, // VblendpsVpsHpsWpsIb -> STATE_AVX
    4, // VblendpdVpdHpdWpdIb -> STATE_AVX
    4, // V128VpblendwVdqHdqWdqIb -> STATE_AVX
    4, // V256VpblendwVdqHdqWdqIb -> STATE_AVX
    4, // V128VpalignrVdqHdqWdqIb -> STATE_AVX
    4, // V256VpalignrVdqHdqWdqIb -> STATE_AVX
    4, // V128VinsertpsVpsWssIb -> STATE_AVX
    4, // V128VextractpsEdVpsIb -> STATE_AVX
    4, // V256Vperm2f128VdqHdqWdqIb -> STATE_AVX
    4, // V256Vinsertf128VdqHdqWdqIb -> STATE_AVX
    4, // V256Vextractf128WdqVdqIb -> STATE_AVX
    4, // VblendvpsVpsHpsWpsIb -> STATE_AVX
    4, // VblendvpdVpdHpdWpdIb -> STATE_AVX
    4, // V128VpblendvbVdqHdqWdqIb -> STATE_AVX
    4, // V256VpblendvbVdqHdqWdqIb -> STATE_AVX
    4, // V128VmpsadbwVdqHdqWdqIb -> STATE_AVX
    4, // V256VmpsadbwVdqHdqWdqIb -> STATE_AVX
    4, // V128VpcmpestrmVdqWdqIb -> STATE_AVX
    4, // V128VpcmpestriVdqWdqIb -> STATE_AVX
    4, // V128VpcmpistrmVdqWdqIb -> STATE_AVX
    4, // V128VpcmpistriVdqWdqIb -> STATE_AVX
    4, // V128VaesimcVdqWdq -> STATE_AVX
    4, // V128VaeskeygenassistVdqWdqIb -> STATE_AVX
    4, // V128VaesencVdqHdqWdq -> STATE_AVX
    4, // V128VaesenclastVdqHdqWdq -> STATE_AVX
    4, // V128VaesdecVdqHdqWdq -> STATE_AVX
    4, // V128VaesdeclastVdqHdqWdq -> STATE_AVX
    4, // V128VpclmulqdqVdqHdqWdqIb -> STATE_AVX
    4, // V256VaesencVdqHdqWdq -> STATE_AVX
    4, // V256VaesenclastVdqHdqWdq -> STATE_AVX
    4, // V256VaesdecVdqHdqWdq -> STATE_AVX
    4, // V256VaesdeclastVdqHdqWdq -> STATE_AVX
    4, // V256VpclmulqdqVdqHdqWdqIb -> STATE_AVX
    4, // Vgf2p8affineqbVdqHdqWdqIb -> STATE_AVX
    4, // Vgf2p8affineinvqbVdqHdqWdqIb -> STATE_AVX
    4, // Vgf2p8mulbVdqHdqWdq -> STATE_AVX
    4, // Vsm3msg1VdqHdqWdq -> STATE_AVX
    4, // Vsm3msg2VdqHdqWdq -> STATE_AVX
    4, // Vsm3rnds2VdqHdqWdqIb -> STATE_AVX
    4, // Vsm4key4VdqHdqWdq -> STATE_AVX
    4, // Vsm4rnds4VdqHdqWdq -> STATE_AVX
    4, // Vsha512msg1VdqWdq -> STATE_AVX
    4, // Vsha512msg2VdqWdq -> STATE_AVX
    4, // Vsha512rnds2VdqHdqWdq -> STATE_AVX
    4, // V128VmovdVdqEd -> STATE_AVX
    4, // V128VmovqVdqEq -> STATE_AVX
    4, // V128VmovdEdVd -> STATE_AVX
    4, // V128VmovqEqVq -> STATE_AVX
    4, // V128VpinsrbVdqEbIb -> STATE_AVX
    4, // V128VpinsrwVdqEwIb -> STATE_AVX
    4, // V128VpextrwGdUdqIb -> STATE_AVX
    4, // V128VpextrbEdVdqIbR -> STATE_AVX
    4, // V128VpextrbMbVdqIbM -> STATE_AVX
    4, // V128VpextrwEdVdqIbR -> STATE_AVX
    4, // V128VpextrwMwVdqIbM -> STATE_AVX
    4, // V128VpinsrdVdqEdIb -> STATE_AVX
    4, // V128VpinsrqVdqEqIb -> STATE_AVX
    4, // V128VpextrdEdVdqIb -> STATE_AVX
    4, // V128VpextrqEqVdqIb -> STATE_AVX
    4, // Vcvtps2pdVpdWps -> STATE_AVX
    4, // Vcvttpd2dqVdqWpd -> STATE_AVX
    4, // Vcvtpd2dqVdqWpd -> STATE_AVX
    4, // Vcvtdq2pdVpdWdq -> STATE_AVX
    4, // Vcvtpd2psVpsWpd -> STATE_AVX
    4, // Vcvtsd2ssVssWsd -> STATE_AVX
    4, // Vcvtss2sdVsdWss -> STATE_AVX
    4, // Vcvtdq2psVpsWdq -> STATE_AVX
    4, // Vcvtps2dqVdqWps -> STATE_AVX
    4, // Vcvttps2dqVdqWps -> STATE_AVX
    4, // Vcvtss2siGdWss -> STATE_AVX
    4, // Vcvtss2siGqWss -> STATE_AVX
    4, // Vcvtsd2siGdWsd -> STATE_AVX
    4, // Vcvtsd2siGqWsd -> STATE_AVX
    4, // Vcvttss2siGdWss -> STATE_AVX
    4, // Vcvttss2siGqWss -> STATE_AVX
    4, // Vcvttsd2siGdWsd -> STATE_AVX
    4, // Vcvttsd2siGqWsd -> STATE_AVX
    4, // Vcvtsi2ssVssEd -> STATE_AVX
    4, // Vcvtsi2ssVssEq -> STATE_AVX
    4, // Vcvtsi2sdVsdEd -> STATE_AVX
    4, // Vcvtsi2sdVsdEq -> STATE_AVX
    4, // VmovqWqVq -> STATE_AVX
    4, // VmovqVqWq -> STATE_AVX
    4, // Vcvtph2psVpsWps -> STATE_AVX
    4, // Vcvtps2phWpsVpsIb -> STATE_AVX
    4, // V256VpmovsxbwVdqWdq -> STATE_AVX
    4, // V256VpmovsxbdVdqWq -> STATE_AVX
    4, // V256VpmovsxbqVdqWd -> STATE_AVX
    4, // V256VpmovsxwdVdqWdq -> STATE_AVX
    4, // V256VpmovsxwqVdqWq -> STATE_AVX
    4, // V256VpmovsxdqVdqWdq -> STATE_AVX
    4, // V256VpmovzxbwVdqWdq -> STATE_AVX
    4, // V256VpmovzxbdVdqWq -> STATE_AVX
    4, // V256VpmovzxbqVdqWd -> STATE_AVX
    4, // V256VpmovzxwdVdqWdq -> STATE_AVX
    4, // V256VpmovzxwqVdqWq -> STATE_AVX
    4, // V256VpmovzxdqVdqWdq -> STATE_AVX
    4, // V256Vperm2i128VdqHdqWdqIb -> STATE_AVX
    4, // V256Vinserti128VdqHdqWdqIb -> STATE_AVX
    4, // V256Vextracti128WdqVdqIb -> STATE_AVX
    4, // V256Vbroadcasti128VdqMdq -> STATE_AVX
    4, // VpbroadcastbVdqWb -> STATE_AVX
    4, // VpbroadcastwVdqWw -> STATE_AVX
    4, // VpbroadcastdVdqWd -> STATE_AVX
    4, // VpbroadcastqVdqWq -> STATE_AVX
    4, // VbroadcastssVpsWss -> STATE_AVX
    4, // V256VbroadcastsdVpdWsd -> STATE_AVX
    4, // VpblenddVdqHdqWdqIb -> STATE_AVX
    4, // VmaskmovdVdqHdqMdq -> STATE_AVX
    4, // VmaskmovqVdqHdqMdq -> STATE_AVX
    4, // VmaskmovdMdqHdqVdq -> STATE_AVX
    4, // VmaskmovqMdqHdqVdq -> STATE_AVX
    4, // VgatherdpsVpsHps -> STATE_AVX
    4, // VgatherdpdVpdHpd -> STATE_AVX
    4, // VgatherqpsVpsHps -> STATE_AVX
    4, // VgatherqpdVpdHpd -> STATE_AVX
    4, // VgatherddVdqHdq -> STATE_AVX
    4, // VgatherdqVdqHdq -> STATE_AVX
    4, // VgatherqdVdqHdq -> STATE_AVX
    4, // VgatherqqVdqHdq -> STATE_AVX
    4, // VpsrlvdVdqHdqWdq -> STATE_AVX
    4, // VpsrlvqVdqHdqWdq -> STATE_AVX
    4, // VpsllvdVdqHdqWdq -> STATE_AVX
    4, // VpsllvqVdqHdqWdq -> STATE_AVX
    4, // V256VpermqVdqWdqIb -> STATE_AVX
    4, // V256VpermdVdqHdqWdq -> STATE_AVX
    4, // V256VpermpsVpsHpsWps -> STATE_AVX
    4, // V256VpermpdVpdWpdIb -> STATE_AVX
    4, // VpsravdVdqHdqWdq -> STATE_AVX
    4, // Vfmadd132psVpsHpsWps -> STATE_AVX
    4, // Vfmadd132pdVpdHpdWpd -> STATE_AVX
    4, // Vfmadd213psVpsHpsWps -> STATE_AVX
    4, // Vfmadd213pdVpdHpdWpd -> STATE_AVX
    4, // Vfmadd231psVpsHpsWps -> STATE_AVX
    4, // Vfmadd231pdVpdHpdWpd -> STATE_AVX
    4, // Vfmadd132ssVpsHssWss -> STATE_AVX
    4, // Vfmadd132sdVpdHsdWsd -> STATE_AVX
    4, // Vfmadd213ssVpsHssWss -> STATE_AVX
    4, // Vfmadd213sdVpdHsdWsd -> STATE_AVX
    4, // Vfmadd231ssVpsHssWss -> STATE_AVX
    4, // Vfmadd231sdVpdHsdWsd -> STATE_AVX
    4, // Vfmaddsub132psVpsHpsWps -> STATE_AVX
    4, // Vfmaddsub132pdVpdHpdWpd -> STATE_AVX
    4, // Vfmaddsub213psVpsHpsWps -> STATE_AVX
    4, // Vfmaddsub213pdVpdHpdWpd -> STATE_AVX
    4, // Vfmaddsub231psVpsHpsWps -> STATE_AVX
    4, // Vfmaddsub231pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsubadd132psVpsHpsWps -> STATE_AVX
    4, // Vfmsubadd132pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsubadd213psVpsHpsWps -> STATE_AVX
    4, // Vfmsubadd213pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsubadd231psVpsHpsWps -> STATE_AVX
    4, // Vfmsubadd231pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsub132psVpsHpsWps -> STATE_AVX
    4, // Vfmsub132pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsub213psVpsHpsWps -> STATE_AVX
    4, // Vfmsub213pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsub231psVpsHpsWps -> STATE_AVX
    4, // Vfmsub231pdVpdHpdWpd -> STATE_AVX
    4, // Vfmsub132ssVpsHssWss -> STATE_AVX
    4, // Vfmsub132sdVpdHsdWsd -> STATE_AVX
    4, // Vfmsub213ssVpsHssWss -> STATE_AVX
    4, // Vfmsub213sdVpdHsdWsd -> STATE_AVX
    4, // Vfmsub231ssVpsHssWss -> STATE_AVX
    4, // Vfmsub231sdVpdHsdWsd -> STATE_AVX
    4, // Vfnmadd132psVpsHpsWps -> STATE_AVX
    4, // Vfnmadd132pdVpdHpdWpd -> STATE_AVX
    4, // Vfnmadd213psVpsHpsWps -> STATE_AVX
    4, // Vfnmadd213pdVpdHpdWpd -> STATE_AVX
    4, // Vfnmadd231psVpsHpsWps -> STATE_AVX
    4, // Vfnmadd231pdVpdHpdWpd -> STATE_AVX
    4, // Vfnmadd132ssVpsHssWss -> STATE_AVX
    4, // Vfnmadd132sdVpdHsdWsd -> STATE_AVX
    4, // Vfnmadd213ssVpsHssWss -> STATE_AVX
    4, // Vfnmadd213sdVpdHsdWsd -> STATE_AVX
    4, // Vfnmadd231ssVpsHssWss -> STATE_AVX
    4, // Vfnmadd231sdVpdHsdWsd -> STATE_AVX
    4, // Vfnmsub132psVpsHpsWps -> STATE_AVX
    4, // Vfnmsub132pdVpdHpdWpd -> STATE_AVX
    4, // Vfnmsub213psVpsHpsWps -> STATE_AVX
    4, // Vfnmsub213pdVpdHpdWpd -> STATE_AVX
    4, // Vfnmsub231psVpsHpsWps -> STATE_AVX
    4, // Vfnmsub231pdVpdHpdWpd -> STATE_AVX
    4, // Vfnmsub132ssVpsHssWss -> STATE_AVX
    4, // Vfnmsub132sdVpdHsdWsd -> STATE_AVX
    4, // Vfnmsub213ssVpsHssWss -> STATE_AVX
    4, // Vfnmsub213sdVpdHsdWsd -> STATE_AVX
    4, // Vfnmsub231ssVpsHssWss -> STATE_AVX
    4, // Vfnmsub231sdVpdHsdWsd -> STATE_AVX
    4, // VpdpbusdVdqHdqWdq -> STATE_AVX
    4, // VpdpbusdsVdqHdqWdq -> STATE_AVX
    4, // VpdpwssdVdqHdqWdq -> STATE_AVX
    4, // VpdpwssdsVdqHdqWdq -> STATE_AVX
    4, // Vpmadd52luqVdqHdqWdq -> STATE_AVX
    4, // Vpmadd52huqVdqHdqWdq -> STATE_AVX
    4, // VpdpbssdVdqHdqWdq -> STATE_AVX
    4, // VpdpbssdsVdqHdqWdq -> STATE_AVX
    4, // VpdpbsudVdqHdqWdq -> STATE_AVX
    4, // VpdpbsudsVdqHdqWdq -> STATE_AVX
    4, // VpdpbuudVdqHdqWdq -> STATE_AVX
    4, // VpdpbuudsVdqHdqWdq -> STATE_AVX
    4, // VpdpwsudVdqHdqWdq -> STATE_AVX
    4, // VpdpwsudsVdqHdqWdq -> STATE_AVX
    4, // VpdpwusdVdqHdqWdq -> STATE_AVX
    4, // VpdpwusdsVdqHdqWdq -> STATE_AVX
    4, // VpdpwuudVdqHdqWdq -> STATE_AVX
    4, // VpdpwuudsVdqHdqWdq -> STATE_AVX
    4, // Vbcstnebf162psVpsWw -> STATE_AVX
    4, // Vbcstnesh2psVpsWsh -> STATE_AVX
    4, // Vcvtneeph2psVpsWph -> STATE_AVX
    4, // Vcvtneoph2psVpsWph -> STATE_AVX
    4, // Vcvtneebf162psVpsWph -> STATE_AVX
    4, // Vcvtneobf162psVpsWph -> STATE_AVX
    4, // Vcvtneps2bf16VphWps -> STATE_AVX
    0, // AndnGdBdEd -> STATE_NONE
    0, // AndnGqBqEq -> STATE_NONE
    0, // BlsiBdEd -> STATE_NONE
    0, // BlsiBqEq -> STATE_NONE
    0, // BlsmskBdEd -> STATE_NONE
    0, // BlsmskBqEq -> STATE_NONE
    0, // BlsrBdEd -> STATE_NONE
    0, // BlsrBqEq -> STATE_NONE
    0, // BextrGdEdBd -> STATE_NONE
    0, // BextrGqEqBq -> STATE_NONE
    0, // MulxGdBdEd -> STATE_NONE
    0, // MulxGqBqEq -> STATE_NONE
    0, // RorxGdEdIb -> STATE_NONE
    0, // RorxGqEqIb -> STATE_NONE
    0, // ShlxGdEdBd -> STATE_NONE
    0, // ShlxGqEqBq -> STATE_NONE
    0, // ShrxGdEdBd -> STATE_NONE
    0, // ShrxGqEqBq -> STATE_NONE
    0, // SarxGdEdBd -> STATE_NONE
    0, // SarxGqEqBq -> STATE_NONE
    0, // BzhiGdBdEd -> STATE_NONE
    0, // BzhiGqBqEq -> STATE_NONE
    0, // PextGdBdEd -> STATE_NONE
    0, // PextGqBqEq -> STATE_NONE
    0, // PdepGdBdEd -> STATE_NONE
    0, // PdepGqBqEq -> STATE_NONE
    0, // CmpbexaddEdGdBd -> STATE_NONE
    0, // CmpbexaddEqGqBq -> STATE_NONE
    0, // CmpbxaddEdGdBd -> STATE_NONE
    0, // CmpbxaddEqGqBq -> STATE_NONE
    0, // CmplexaddEdGdBd -> STATE_NONE
    0, // CmplexaddEqGqBq -> STATE_NONE
    0, // CmplxaddEdGdBd -> STATE_NONE
    0, // CmplxaddEqGqBq -> STATE_NONE
    0, // CmpnbexaddEdGdBd -> STATE_NONE
    0, // CmpnbexaddEqGqBq -> STATE_NONE
    0, // CmpnbxaddEdGdBd -> STATE_NONE
    0, // CmpnbxaddEqGqBq -> STATE_NONE
    0, // CmpnlexaddEdGdBd -> STATE_NONE
    0, // CmpnlexaddEqGqBq -> STATE_NONE
    0, // CmpnlxaddEdGdBd -> STATE_NONE
    0, // CmpnlxaddEqGqBq -> STATE_NONE
    0, // CmpnoxaddEdGdBd -> STATE_NONE
    0, // CmpnoxaddEqGqBq -> STATE_NONE
    0, // CmpnpxaddEdGdBd -> STATE_NONE
    0, // CmpnpxaddEqGqBq -> STATE_NONE
    0, // CmpnsxaddEdGdBd -> STATE_NONE
    0, // CmpnsxaddEqGqBq -> STATE_NONE
    0, // CmpnzxaddEdGdBd -> STATE_NONE
    0, // CmpnzxaddEqGqBq -> STATE_NONE
    0, // CmpoxaddEdGdBd -> STATE_NONE
    0, // CmpoxaddEqGqBq -> STATE_NONE
    0, // CmppxaddEdGdBd -> STATE_NONE
    0, // CmppxaddEqGqBq -> STATE_NONE
    0, // CmpsxaddEdGdBd -> STATE_NONE
    0, // CmpsxaddEqGqBq -> STATE_NONE
    0, // CmpzxaddEdGdBd -> STATE_NONE
    0, // CmpzxaddEqGqBq -> STATE_NONE
    4, // VfmaddsubpsVpsHpsVibWps -> STATE_AVX
    4, // VfmaddsubpsVpsHpsWpsVib -> STATE_AVX
    4, // VfmaddsubpdVpdHpdVibWpd -> STATE_AVX
    4, // VfmaddsubpdVpdHpdWpdVib -> STATE_AVX
    4, // VfmsubaddpsVpsHpsVibWps -> STATE_AVX
    4, // VfmsubaddpsVpsHpsWpsVib -> STATE_AVX
    4, // VfmsubaddpdVpdHpdVibWpd -> STATE_AVX
    4, // VfmsubaddpdVpdHpdWpdVib -> STATE_AVX
    4, // VfmaddpsVpsHpsVibWps -> STATE_AVX
    4, // VfmaddpsVpsHpsWpsVib -> STATE_AVX
    4, // VfmaddpdVpdHpdVibWpd -> STATE_AVX
    4, // VfmaddpdVpdHpdWpdVib -> STATE_AVX
    4, // VfmaddssVssHssVibWss -> STATE_AVX
    4, // VfmaddssVssHssWssVib -> STATE_AVX
    4, // VfmaddsdVsdHsdVibWsd -> STATE_AVX
    4, // VfmaddsdVsdHsdWsdVib -> STATE_AVX
    4, // VfmsubpsVpsHpsVibWps -> STATE_AVX
    4, // VfmsubpsVpsHpsWpsVib -> STATE_AVX
    4, // VfmsubpdVpdHpdVibWpd -> STATE_AVX
    4, // VfmsubpdVpdHpdWpdVib -> STATE_AVX
    4, // VfmsubssVssHssVibWss -> STATE_AVX
    4, // VfmsubssVssHssWssVib -> STATE_AVX
    4, // VfmsubsdVsdHsdVibWsd -> STATE_AVX
    4, // VfmsubsdVsdHsdWsdVib -> STATE_AVX
    4, // VfnmaddpsVpsHpsVibWps -> STATE_AVX
    4, // VfnmaddpsVpsHpsWpsVib -> STATE_AVX
    4, // VfnmaddpdVpdHpdVibWpd -> STATE_AVX
    4, // VfnmaddpdVpdHpdWpdVib -> STATE_AVX
    4, // VfnmaddssVssHssVibWss -> STATE_AVX
    4, // VfnmaddssVssHssWssVib -> STATE_AVX
    4, // VfnmaddsdVsdHsdVibWsd -> STATE_AVX
    4, // VfnmaddsdVsdHsdWsdVib -> STATE_AVX
    4, // VfnmsubpsVpsHpsVibWps -> STATE_AVX
    4, // VfnmsubpsVpsHpsWpsVib -> STATE_AVX
    4, // VfnmsubpdVpdHpdVibWpd -> STATE_AVX
    4, // VfnmsubpdVpdHpdWpdVib -> STATE_AVX
    4, // VfnmsubssVssHssVibWss -> STATE_AVX
    4, // VfnmsubssVssHssWssVib -> STATE_AVX
    4, // VfnmsubsdVsdHsdVibWsd -> STATE_AVX
    4, // VfnmsubsdVsdHsdWsdVib -> STATE_AVX
    4, // VpcmovVdqHdqVibWdq -> STATE_AVX
    4, // VpcmovVdqHdqWdqVib -> STATE_AVX
    4, // VppermVdqHdqVibWdq -> STATE_AVX
    4, // VppermVdqHdqWdqVib -> STATE_AVX
    4, // Vpermil2psVdqHdqVibWdq -> STATE_AVX
    4, // Vpermil2psVdqHdqWdqVib -> STATE_AVX
    4, // Vpermil2pdVdqHdqVibWdq -> STATE_AVX
    4, // Vpermil2pdVdqHdqWdqVib -> STATE_AVX
    4, // VpshabVdqHdqWdq -> STATE_AVX
    4, // VpshabVdqWdqHdq -> STATE_AVX
    4, // VpshawVdqHdqWdq -> STATE_AVX
    4, // VpshawVdqWdqHdq -> STATE_AVX
    4, // VpshadVdqHdqWdq -> STATE_AVX
    4, // VpshadVdqWdqHdq -> STATE_AVX
    4, // VpshaqVdqHdqWdq -> STATE_AVX
    4, // VpshaqVdqWdqHdq -> STATE_AVX
    4, // VprotbVdqHdqWdq -> STATE_AVX
    4, // VprotbVdqWdqHdq -> STATE_AVX
    4, // VprotwVdqHdqWdq -> STATE_AVX
    4, // VprotwVdqWdqHdq -> STATE_AVX
    4, // VprotdVdqHdqWdq -> STATE_AVX
    4, // VprotdVdqWdqHdq -> STATE_AVX
    4, // VprotqVdqHdqWdq -> STATE_AVX
    4, // VprotqVdqWdqHdq -> STATE_AVX
    4, // VpshlbVdqHdqWdq -> STATE_AVX
    4, // VpshlbVdqWdqHdq -> STATE_AVX
    4, // VpshlwVdqHdqWdq -> STATE_AVX
    4, // VpshlwVdqWdqHdq -> STATE_AVX
    4, // VpshldVdqHdqWdq -> STATE_AVX
    4, // VpshldVdqWdqHdq -> STATE_AVX
    4, // VpshlqVdqHdqWdq -> STATE_AVX
    4, // VpshlqVdqWdqHdq -> STATE_AVX
    4, // VpmacsswwVdqHdqWdqVib -> STATE_AVX
    4, // VpmacsswdVdqHdqWdqVib -> STATE_AVX
    4, // VpmacssdqlVdqHdqWdqVib -> STATE_AVX
    4, // VpmacssddVdqHdqWdqVib -> STATE_AVX
    4, // VpmacssdqhVdqHdqWdqVib -> STATE_AVX
    4, // VpmacswwVdqHdqWdqVib -> STATE_AVX
    4, // VpmacswdVdqHdqWdqVib -> STATE_AVX
    4, // VpmacsdqlVdqHdqWdqVib -> STATE_AVX
    4, // VpmacsddVdqHdqWdqVib -> STATE_AVX
    4, // VpmacsdqhVdqHdqWdqVib -> STATE_AVX
    4, // VpmadcsswdVdqHdqWdqVib -> STATE_AVX
    4, // VpmadcswdVdqHdqWdqVib -> STATE_AVX
    4, // VprotbVdqWdqIb -> STATE_AVX
    4, // VprotwVdqWdqIb -> STATE_AVX
    4, // VprotdVdqWdqIb -> STATE_AVX
    4, // VprotqVdqWdqIb -> STATE_AVX
    4, // VpcombVdqHdqWdqIb -> STATE_AVX
    4, // VpcomwVdqHdqWdqIb -> STATE_AVX
    4, // VpcomdVdqHdqWdqIb -> STATE_AVX
    4, // VpcomqVdqHdqWdqIb -> STATE_AVX
    4, // VpcomubVdqHdqWdqIb -> STATE_AVX
    4, // VpcomuwVdqHdqWdqIb -> STATE_AVX
    4, // VpcomudVdqHdqWdqIb -> STATE_AVX
    4, // VpcomuqVdqHdqWdqIb -> STATE_AVX
    4, // VfrczpsVpsWps -> STATE_AVX
    4, // VfrczpdVpdWpd -> STATE_AVX
    4, // VfrczssVssWss -> STATE_AVX
    4, // VfrczsdVsdWsd -> STATE_AVX
    4, // VphaddbwVdqWdq -> STATE_AVX
    4, // VphaddbdVdqWdq -> STATE_AVX
    4, // VphaddbqVdqWdq -> STATE_AVX
    4, // VphaddwdVdqWdq -> STATE_AVX
    4, // VphaddwqVdqWdq -> STATE_AVX
    4, // VphadddqVdqWdq -> STATE_AVX
    4, // VphaddubwVdqWdq -> STATE_AVX
    4, // VphaddubdVdqWdq -> STATE_AVX
    4, // VphaddubqVdqWdq -> STATE_AVX
    4, // VphadduwdVdqWdq -> STATE_AVX
    4, // VphadduwqVdqWdq -> STATE_AVX
    4, // VphaddudqVdqWdq -> STATE_AVX
    4, // VphsubbwVdqWdq -> STATE_AVX
    4, // VphsubwdVdqWdq -> STATE_AVX
    4, // VphsubdqVdqWdq -> STATE_AVX
    0, // BextrGdEdId -> STATE_NONE
    0, // BextrGqEqId -> STATE_NONE
    0, // BlcfillBdEd -> STATE_NONE
    0, // BlcfillBqEq -> STATE_NONE
    0, // BlciBdEd -> STATE_NONE
    0, // BlciBqEq -> STATE_NONE
    0, // BlcicBdEd -> STATE_NONE
    0, // BlcicBqEq -> STATE_NONE
    0, // BlcmskBdEd -> STATE_NONE
    0, // BlcmskBqEq -> STATE_NONE
    0, // BlcsBdEd -> STATE_NONE
    0, // BlcsBqEq -> STATE_NONE
    0, // BlsfillBdEd -> STATE_NONE
    0, // BlsfillBqEq -> STATE_NONE
    0, // BlsicBdEd -> STATE_NONE
    0, // BlsicBqEq -> STATE_NONE
    0, // T1mskcBdEd -> STATE_NONE
    0, // T1mskcBqEq -> STATE_NONE
    0, // TzmskBdEd -> STATE_NONE
    0, // TzmskBqEq -> STATE_NONE
    0, // TzcntGwEw -> STATE_NONE
    0, // TzcntGdEd -> STATE_NONE
    0, // TzcntGqEq -> STATE_NONE
    0, // LzcntGwEw -> STATE_NONE
    0, // LzcntGdEd -> STATE_NONE
    0, // LzcntGqEq -> STATE_NONE
    3, // MovntssMssVss -> STATE_SSE
    3, // MovntsdMsdVsd -> STATE_SSE
    3, // ExtrqUdqIbIb -> STATE_SSE
    3, // ExtrqVdqUq -> STATE_SSE
    3, // InsertqVdqUqIbIb -> STATE_SSE
    3, // InsertqVdqUdq -> STATE_SSE
    0, // AdcxGdEd -> STATE_NONE
    0, // AdoxGdEd -> STATE_NONE
    0, // AdcxGqEq -> STATE_NONE
    0, // AdoxGqEq -> STATE_NONE
    0, // Stac -> STATE_NONE
    0, // Clac -> STATE_NONE
    0, // RdrandEw -> STATE_NONE
    0, // RdrandEd -> STATE_NONE
    0, // RdrandEq -> STATE_NONE
    0, // RdseedEw -> STATE_NONE
    0, // RdseedEd -> STATE_NONE
    0, // RdseedEq -> STATE_NONE
    0, // MovdiriMdGd -> STATE_NONE
    0, // MovdiriMqGq -> STATE_NONE
    0, // Movdir64bGdMdq -> STATE_NONE
    0, // Movdir64bGqMdq -> STATE_NONE
    0, // AaddEdGd -> STATE_NONE
    0, // AandEdGd -> STATE_NONE
    0, // AorEdGd -> STATE_NONE
    0, // AxorEdGd -> STATE_NONE
    0, // AaddEqGq -> STATE_NONE
    0, // AandEqGq -> STATE_NONE
    0, // AorEqGq -> STATE_NONE
    0, // AxorEqGq -> STATE_NONE
    6, // Ldtilecfg -> STATE_AMX
    6, // Sttilecfg -> STATE_AMX
    6, // TileloaddTnnnMdq -> STATE_AMX
    6, // Tileloaddt1TnnnMdq -> STATE_AMX
    6, // TileloaddrsTnnnMdq -> STATE_AMX
    6, // Tileloaddrst1TnnnMdq -> STATE_AMX
    6, // TilestoredMdqTnnn -> STATE_AMX
    6, // Tilerelease -> STATE_AMX
    6, // TilezeroTnnn -> STATE_AMX
    6, // TdpbssdTnnnTrmTreg -> STATE_AMX
    6, // TdpbsudTnnnTrmTreg -> STATE_AMX
    6, // TdpbusdTnnnTrmTreg -> STATE_AMX
    6, // TdpbuudTnnnTrmTreg -> STATE_AMX
    6, // Tdpbf16psTnnnTrmTreg -> STATE_AMX
    6, // Tdpfp16psTnnnTrmTreg -> STATE_AMX
    6, // Tcmmrlfp16psTnnnTrmTreg -> STATE_AMX
    6, // Tcmmimfp16psTnnnTrmTreg -> STATE_AMX
    0, // Tmmultf32psTnnnTrmTreg -> STATE_NONE
    6, // Tdpbf8psTnnnTrmTreg -> STATE_AMX
    6, // Tdphf8psTnnnTrmTreg -> STATE_AMX
    6, // Tdpbhf8psTnnnTrmTreg -> STATE_AMX
    6, // Tdphbf8psTnnnTrmTreg -> STATE_AMX
    5, // KaddwKgwKhwKew -> STATE_EVEX
    5, // KaddqKgqKhqKeq -> STATE_EVEX
    5, // KaddbKgbKhbKeb -> STATE_EVEX
    5, // KadddKgdKhdKed -> STATE_EVEX
    5, // KandwKgwKhwKew -> STATE_EVEX
    5, // KandqKgqKhqKeq -> STATE_EVEX
    5, // KandbKgbKhbKeb -> STATE_EVEX
    5, // KanddKgdKhdKed -> STATE_EVEX
    5, // KandnwKgwKhwKew -> STATE_EVEX
    5, // KandnqKgqKhqKeq -> STATE_EVEX
    5, // KandnbKgbKhbKeb -> STATE_EVEX
    5, // KandndKgdKhdKed -> STATE_EVEX
    5, // KmovwKgwKew -> STATE_EVEX
    5, // KmovqKgqKeq -> STATE_EVEX
    5, // KmovbKgbKeb -> STATE_EVEX
    5, // KmovdKgdKed -> STATE_EVEX
    5, // KmovwKewKgw -> STATE_EVEX
    5, // KmovqKeqKgq -> STATE_EVEX
    5, // KmovbKebKgb -> STATE_EVEX
    5, // KmovdKedKgd -> STATE_EVEX
    5, // KmovbGdKeb -> STATE_EVEX
    5, // KmovwGdKew -> STATE_EVEX
    5, // KmovdGdKed -> STATE_EVEX
    5, // KmovqGqKeq -> STATE_EVEX
    5, // KmovbKgbEb -> STATE_EVEX
    5, // KmovwKgwEw -> STATE_EVEX
    5, // KmovdKgdEd -> STATE_EVEX
    5, // KmovqKgqEq -> STATE_EVEX
    5, // KunpckbwKgwKhbKeb -> STATE_EVEX
    5, // KunpckwdKgdKhwKew -> STATE_EVEX
    5, // KunpckdqKgqKhdKed -> STATE_EVEX
    5, // KnotwKgwKew -> STATE_EVEX
    5, // KnotqKgqKeq -> STATE_EVEX
    5, // KnotbKgbKeb -> STATE_EVEX
    5, // KnotdKgdKed -> STATE_EVEX
    5, // KorwKgwKhwKew -> STATE_EVEX
    5, // KorqKgqKhqKeq -> STATE_EVEX
    5, // KorbKgbKhbKeb -> STATE_EVEX
    5, // KordKgdKhdKed -> STATE_EVEX
    5, // KortestwKgwKew -> STATE_EVEX
    5, // KortestqKgqKeq -> STATE_EVEX
    5, // KortestbKgbKeb -> STATE_EVEX
    5, // KortestdKgdKed -> STATE_EVEX
    5, // KshiftlbKgbKebIb -> STATE_EVEX
    5, // KshiftlwKgwKewIb -> STATE_EVEX
    5, // KshiftldKgdKedIb -> STATE_EVEX
    5, // KshiftlqKgqKeqIb -> STATE_EVEX
    5, // KshiftrbKgbKebIb -> STATE_EVEX
    5, // KshiftrwKgwKewIb -> STATE_EVEX
    5, // KshiftrdKgdKedIb -> STATE_EVEX
    5, // KshiftrqKgqKeqIb -> STATE_EVEX
    5, // KxnorwKgwKhwKew -> STATE_EVEX
    5, // KxnorqKgqKhqKeq -> STATE_EVEX
    5, // KxnorbKgbKhbKeb -> STATE_EVEX
    5, // KxnordKgdKhdKed -> STATE_EVEX
    5, // KxorwKgwKhwKew -> STATE_EVEX
    5, // KxorqKgqKhqKeq -> STATE_EVEX
    5, // KxorbKgbKhbKeb -> STATE_EVEX
    5, // KxordKgdKhdKed -> STATE_EVEX
    5, // KtestwKgwKew -> STATE_EVEX
    5, // KtestqKgqKeq -> STATE_EVEX
    5, // KtestbKgbKeb -> STATE_EVEX
    5, // KtestdKgdKed -> STATE_EVEX
    0, // RdmsrEqId -> STATE_NONE
    0, // WrmsrnsIdEq -> STATE_NONE
    0, // MovrsGbEb -> STATE_NONE
    0, // MovrsGwEw -> STATE_NONE
    0, // MovrsGdEd -> STATE_NONE
    0, // MovrsGqEq -> STATE_NONE
    0, // Erets -> STATE_NONE
    0, // Eretu -> STATE_NONE
    0, // LkgsEw -> STATE_NONE
    5, // EvexVaddpsVpsHpsWps -> STATE_EVEX
    5, // EvexVaddpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVaddssVssHpsWss -> STATE_EVEX
    5, // EvexVaddsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVaddpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVaddpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVaddssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVaddsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVsubpsVpsHpsWps -> STATE_EVEX
    5, // EvexVsubpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVsubssVssHpsWss -> STATE_EVEX
    5, // EvexVsubsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVsubpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVsubpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVsubssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVsubsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVmulpsVpsHpsWps -> STATE_EVEX
    5, // EvexVmulpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVmulssVssHpsWss -> STATE_EVEX
    5, // EvexVmulsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVmulpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVmulpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVmulssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVmulsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVdivpsVpsHpsWps -> STATE_EVEX
    5, // EvexVdivpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVdivssVssHpsWss -> STATE_EVEX
    5, // EvexVdivsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVdivpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVdivpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVdivssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVdivsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVminpsVpsHpsWps -> STATE_EVEX
    5, // EvexVminpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVminssVssHpsWss -> STATE_EVEX
    5, // EvexVminsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVminpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVminpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVminssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVminsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVmaxpsVpsHpsWps -> STATE_EVEX
    5, // EvexVmaxpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVmaxssVssHpsWss -> STATE_EVEX
    5, // EvexVmaxsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVmaxpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVmaxpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVmaxssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVmaxsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVsqrtpsVpsWps -> STATE_EVEX
    5, // EvexVsqrtpdVpdWpd -> STATE_EVEX
    5, // EvexVsqrtssVssHpsWss -> STATE_EVEX
    5, // EvexVsqrtsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVsqrtpsVpsWpsKmask -> STATE_EVEX
    5, // EvexVsqrtpdVpdWpdKmask -> STATE_EVEX
    5, // EvexVsqrtssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVsqrtsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVcmppsKgwHpsWpsIb -> STATE_EVEX
    5, // EvexVcmppdKgbHpdWpdIb -> STATE_EVEX
    5, // EvexVcmpssKgbHssWssIb -> STATE_EVEX
    5, // EvexVcmpsdKgbHsdWsdIb -> STATE_EVEX
    5, // EvexVrndscalepsVpsWpsIbKmask -> STATE_EVEX
    5, // EvexVrndscalepdVpdWpdIbKmask -> STATE_EVEX
    5, // EvexVrndscalessVssHpsWssIbKmask -> STATE_EVEX
    5, // EvexVrndscalesdVsdHpdWsdIbKmask -> STATE_EVEX
    5, // EvexVunpcklpsVpsHpsWps -> STATE_EVEX
    5, // EvexVunpcklpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVunpcklpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVunpcklpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVunpckhpsVpsHpsWps -> STATE_EVEX
    5, // EvexVunpckhpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVunpckhpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVunpckhpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVpunpckldqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpcklqdqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpckldqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpcklqdqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpckhdqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpckhqdqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpckhdqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpckhqdqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmuldqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmuludqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmuldqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmuludqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVucomissVssWss -> STATE_EVEX
    5, // EvexVcomissVssWss -> STATE_EVEX
    5, // EvexVucomisdVsdWsd -> STATE_EVEX
    5, // EvexVcomisdVsdWsd -> STATE_EVEX
    5, // EvexVcvtss2sdVsdWss -> STATE_EVEX
    5, // EvexVcvtsd2ssVssWsd -> STATE_EVEX
    5, // EvexVcvtps2pdVpdWps -> STATE_EVEX
    5, // EvexVcvtpd2psVpsWpd -> STATE_EVEX
    5, // EvexVcvtss2sdVsdWssKmask -> STATE_EVEX
    5, // EvexVcvtsd2ssVssWsdKmask -> STATE_EVEX
    5, // EvexVcvtps2pdVpdWpsKmask -> STATE_EVEX
    5, // EvexVcvtpd2psVpsWpdKmask -> STATE_EVEX
    5, // EvexVcvtps2dqVdqWps -> STATE_EVEX
    5, // EvexVcvtps2dqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttps2dqVdqWps -> STATE_EVEX
    5, // EvexVcvttps2dqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvtpd2dqVdqWpd -> STATE_EVEX
    5, // EvexVcvtpd2dqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttpd2dqVdqWpd -> STATE_EVEX
    5, // EvexVcvttpd2dqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvtph2psVpsWps -> STATE_EVEX
    5, // EvexVcvtph2psVpsWpsKmask -> STATE_EVEX
    5, // EvexVcvtps2phWpsVpsIb -> STATE_EVEX
    5, // EvexVcvtps2phWpsVpsIbKmask -> STATE_EVEX
    5, // EvexVcvtneps2bf16VphWpsKmask -> STATE_EVEX
    5, // EvexVcvtne2ps2bf16VphHpsWpsKmask -> STATE_EVEX
    5, // EvexVdpbf16psVpsHdqWdqKmask -> STATE_EVEX
    5, // EvexVmovapsVpsWps -> STATE_EVEX
    5, // EvexVmovapsVpsWpsKmask -> STATE_EVEX
    5, // EvexVmovapsWpsVps -> STATE_EVEX
    5, // EvexVmovapsWpsVpsKmask -> STATE_EVEX
    5, // EvexVmovapdVpdWpd -> STATE_EVEX
    5, // EvexVmovapdVpdWpdKmask -> STATE_EVEX
    5, // EvexVmovapdWpdVpd -> STATE_EVEX
    5, // EvexVmovapdWpdVpdKmask -> STATE_EVEX
    5, // EvexVmovupsVpsWps -> STATE_EVEX
    5, // EvexVmovupsVpsWpsKmask -> STATE_EVEX
    5, // EvexVmovupsWpsVps -> STATE_EVEX
    5, // EvexVmovupsWpsVpsKmask -> STATE_EVEX
    5, // EvexVmovupdVpdWpd -> STATE_EVEX
    5, // EvexVmovupdVpdWpdKmask -> STATE_EVEX
    5, // EvexVmovupdWpdVpd -> STATE_EVEX
    5, // EvexVmovupdWpdVpdKmask -> STATE_EVEX
    5, // EvexVmovsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVmovssVssHpsWss -> STATE_EVEX
    5, // EvexVmovsdWsdHpdVsd -> STATE_EVEX
    5, // EvexVmovssWssHpsVss -> STATE_EVEX
    5, // EvexVmovsdVsdWsd -> STATE_EVEX
    5, // EvexVmovssVssWss -> STATE_EVEX
    5, // EvexVmovsdWsdVsd -> STATE_EVEX
    5, // EvexVmovssWssVss -> STATE_EVEX
    5, // EvexVmovsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVmovssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVmovsdWsdHpdVsdKmask -> STATE_EVEX
    5, // EvexVmovssWssHpsVssKmask -> STATE_EVEX
    5, // EvexVmovsdVsdWsdKmask -> STATE_EVEX
    5, // EvexVmovssVssWssKmask -> STATE_EVEX
    5, // EvexVmovsdWsdVsdKmask -> STATE_EVEX
    5, // EvexVmovssWssVssKmask -> STATE_EVEX
    5, // EvexVpabsbVdqWdq -> STATE_EVEX
    5, // EvexVpabswVdqWdq -> STATE_EVEX
    5, // EvexVpabsdVdqWdq -> STATE_EVEX
    5, // EvexVpabsqVdqWdq -> STATE_EVEX
    5, // EvexVpabsbVdqWdqKmask -> STATE_EVEX
    5, // EvexVpabswVdqWdqKmask -> STATE_EVEX
    5, // EvexVpabsdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpabsqVdqWdqKmask -> STATE_EVEX
    5, // EvexVmovntdqaVdqMdq -> STATE_EVEX
    5, // EvexVmovntpsMpsVps -> STATE_EVEX
    5, // EvexVmovntpdMpdVpd -> STATE_EVEX
    5, // EvexVmovntdqMdqVdq -> STATE_EVEX
    5, // EvexVpcmpeqbKgqHdqWdq -> STATE_EVEX
    5, // EvexVpcmpeqwKgdHdqWdq -> STATE_EVEX
    5, // EvexVpcmpgtbKgqHdqWdq -> STATE_EVEX
    5, // EvexVpcmpgtwKgdHdqWdq -> STATE_EVEX
    5, // EvexVpcmpeqdKgwHdqWdq -> STATE_EVEX
    5, // EvexVpcmpeqqKgbHdqWdq -> STATE_EVEX
    5, // EvexVpcmpgtdKgwHdqWdq -> STATE_EVEX
    5, // EvexVpcmpgtqKgbHdqWdq -> STATE_EVEX
    5, // EvexVpsrlwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrlwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrawVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrawVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsllwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsllwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrlwUdqIb -> STATE_EVEX
    5, // EvexVpsrlwUdqIbKmask -> STATE_EVEX
    5, // EvexVpsllwUdqIb -> STATE_EVEX
    5, // EvexVpsllwUdqIbKmask -> STATE_EVEX
    5, // EvexVpsrawUdqIb -> STATE_EVEX
    5, // EvexVpsrawUdqIbKmask -> STATE_EVEX
    5, // EvexVpsrldVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrlqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrldVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrlqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpslldVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsllqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpslldVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsllqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrldUdqIb -> STATE_EVEX
    5, // EvexVpsrldUdqIbKmask -> STATE_EVEX
    5, // EvexVpsrlqUdqIb -> STATE_EVEX
    5, // EvexVpsrlqUdqIbKmask -> STATE_EVEX
    5, // EvexVpslldUdqIb -> STATE_EVEX
    5, // EvexVpslldUdqIbKmask -> STATE_EVEX
    5, // EvexVpsllqUdqIb -> STATE_EVEX
    5, // EvexVpsllqUdqIbKmask -> STATE_EVEX
    5, // EvexVpshufbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpshufbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermqVdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpermpdVpdWpdIbKmask -> STATE_EVEX
    5, // EvexVshufpsVpsHpsWpsIb -> STATE_EVEX
    5, // EvexVshufpdVpdHpdWpdIb -> STATE_EVEX
    5, // EvexVshufpsVpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVshufpdVpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVpermilpsVpsHpsWps -> STATE_EVEX
    5, // EvexVpermilpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVpermilpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVpermilpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVpermilpsVpsWpsIb -> STATE_EVEX
    5, // EvexVpermilpdVpdWpdIb -> STATE_EVEX
    5, // EvexVpermilpsVpsWpsIbKmask -> STATE_EVEX
    5, // EvexVpermilpdVpdWpdIbKmask -> STATE_EVEX
    5, // EvexVpshufdVdqWdqIb -> STATE_EVEX
    5, // EvexVpshufdVdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshuflwVdqWdqIb -> STATE_EVEX
    5, // EvexVpshuflwVdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshufhwVdqWdqIb -> STATE_EVEX
    5, // EvexVpshufhwVdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpbroadcastbVdqEb -> STATE_EVEX
    5, // EvexVpbroadcastbVdqEbKmask -> STATE_EVEX
    5, // EvexVpbroadcastwVdqEw -> STATE_EVEX
    5, // EvexVpbroadcastwVdqEwKmask -> STATE_EVEX
    5, // EvexVpbroadcastdVdqEd -> STATE_EVEX
    5, // EvexVpbroadcastdVdqEdKmask -> STATE_EVEX
    5, // EvexVpbroadcastqVdqEq -> STATE_EVEX
    5, // EvexVpbroadcastqVdqEqKmask -> STATE_EVEX
    5, // EvexVpbroadcastbVdqWb -> STATE_EVEX
    5, // EvexVpbroadcastbVdqWbKmask -> STATE_EVEX
    5, // EvexVpbroadcastwVdqWw -> STATE_EVEX
    5, // EvexVpbroadcastwVdqWwKmask -> STATE_EVEX
    5, // EvexVpbroadcastdVdqWd -> STATE_EVEX
    5, // EvexVpbroadcastdVdqWdKmask -> STATE_EVEX
    5, // EvexVpbroadcastqVdqWq -> STATE_EVEX
    5, // EvexVpbroadcastqVdqWqKmask -> STATE_EVEX
    5, // EvexVbroadcastssVpsWss -> STATE_EVEX
    5, // EvexVbroadcastssVpsWssKmask -> STATE_EVEX
    5, // EvexVbroadcastsdVpdWsd -> STATE_EVEX
    5, // EvexVbroadcastsdVpdWsdKmask -> STATE_EVEX
    5, // EvexVmovqWqVq -> STATE_EVEX
    5, // EvexVmovqVqWq -> STATE_EVEX
    5, // EvexVinsertpsVpsWssIb -> STATE_EVEX
    5, // EvexVextractpsEdVpsIb -> STATE_EVEX
    5, // EvexVmovlpsVpsHpsMq -> STATE_EVEX
    5, // EvexVmovhlpsVpsHpsWps -> STATE_EVEX
    5, // EvexVmovhpsVpsHpsMq -> STATE_EVEX
    5, // EvexVmovlhpsVpsHpsWps -> STATE_EVEX
    5, // EvexVmovlpsMqVps -> STATE_EVEX
    5, // EvexVmovhpsMqVps -> STATE_EVEX
    5, // EvexVmovlpdMqVsd -> STATE_EVEX
    5, // EvexVmovhpdMqVsd -> STATE_EVEX
    5, // EvexVmovlpdVpdHpdMq -> STATE_EVEX
    5, // EvexVmovhpdVpdHpdMq -> STATE_EVEX
    5, // EvexVmovddupVpdWpd -> STATE_EVEX
    5, // EvexVmovsldupVpsWps -> STATE_EVEX
    5, // EvexVmovshdupVpsWps -> STATE_EVEX
    5, // EvexVmovddupVpdWpdKmask -> STATE_EVEX
    5, // EvexVmovsldupVpsWpsKmask -> STATE_EVEX
    5, // EvexVmovshdupVpsWpsKmask -> STATE_EVEX
    5, // EvexVpmovqbWdqVdq -> STATE_EVEX
    5, // EvexVpmovdbWdqVdq -> STATE_EVEX
    5, // EvexVpmovwbWdqVdq -> STATE_EVEX
    5, // EvexVpmovdwWdqVdq -> STATE_EVEX
    5, // EvexVpmovqwWdqVdq -> STATE_EVEX
    5, // EvexVpmovqdWdqVdq -> STATE_EVEX
    5, // EvexVpmovqbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovdbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovwbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovdwWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovqwWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovqdWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovusqbWdqVdq -> STATE_EVEX
    5, // EvexVpmovusdbWdqVdq -> STATE_EVEX
    5, // EvexVpmovuswbWdqVdq -> STATE_EVEX
    5, // EvexVpmovusdwWdqVdq -> STATE_EVEX
    5, // EvexVpmovusqwWdqVdq -> STATE_EVEX
    5, // EvexVpmovusqdWdqVdq -> STATE_EVEX
    5, // EvexVpmovusqbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovusdbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovuswbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovusdwWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovusqwWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovusqdWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovsqbWdqVdq -> STATE_EVEX
    5, // EvexVpmovsdbWdqVdq -> STATE_EVEX
    5, // EvexVpmovswbWdqVdq -> STATE_EVEX
    5, // EvexVpmovsdwWdqVdq -> STATE_EVEX
    5, // EvexVpmovsqwWdqVdq -> STATE_EVEX
    5, // EvexVpmovsqdWdqVdq -> STATE_EVEX
    5, // EvexVpmovsqbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovsdbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovswbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovsdwWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovsqwWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovsqdWdqVdqKmask -> STATE_EVEX
    5, // EvexVpmovsxbwVdqWdq -> STATE_EVEX
    5, // EvexVpmovsxbdVdqWdq -> STATE_EVEX
    5, // EvexVpmovsxbqVdqWdq -> STATE_EVEX
    5, // EvexVpmovsxwdVdqWdq -> STATE_EVEX
    5, // EvexVpmovsxwqVdqWdq -> STATE_EVEX
    5, // EvexVpmovsxdqVdqWdq -> STATE_EVEX
    5, // EvexVpmovsxbwVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovsxbdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovsxbqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovsxwdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovsxwqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovsxdqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovzxbwVdqWdq -> STATE_EVEX
    5, // EvexVpmovzxbdVdqWdq -> STATE_EVEX
    5, // EvexVpmovzxbqVdqWdq -> STATE_EVEX
    5, // EvexVpmovzxwdVdqWdq -> STATE_EVEX
    5, // EvexVpmovzxwqVdqWdq -> STATE_EVEX
    5, // EvexVpmovzxdqVdqWdq -> STATE_EVEX
    5, // EvexVpmovzxbwVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovzxbdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovzxbqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovzxwdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovzxwqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovzxdqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubsbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubusbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubuswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpaddbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpaddsbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpaddusbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpaddwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpaddswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpadduswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubsbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubusbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubuswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpaddbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpaddsbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpaddusbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpaddwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpaddswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpadduswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminsbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminubVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxubVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxsbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminuwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxuwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminsbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminubVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxubVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxsbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminuwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxuwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpacksswbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpacksswbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpackuswbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpackuswbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpackssdwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpackssdwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpackusdwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpackusdwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpcklbwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpckhbwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpcklbwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpckhbwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpcklwdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpckhwdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpunpcklwdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpunpckhwdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpavgbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpavgwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpavgbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpavgwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaddubswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaddubswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmullwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmulhwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmulhuwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmulhrswVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmullwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmulhwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmulhuwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmulhrswVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrldqUdqIb -> STATE_EVEX
    5, // EvexVpslldqUdqIb -> STATE_EVEX
    5, // EvexVpsadbwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaddwdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaddwdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmadd52luqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmadd52luqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmadd52huqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmadd52huqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmultishiftqbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmultishiftqbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermt2bVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermt2wVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermi2bVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermi2wVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVinsertf32x4VpsHpsWpsIb -> STATE_EVEX
    5, // EvexVinsertf64x2VpdHpdWpdIb -> STATE_EVEX
    5, // EvexVinsertf32x4VpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVinsertf64x2VpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVinsertf32x8VpsHpsWpsIb -> STATE_EVEX
    5, // EvexVinsertf64x4VpdHpdWpdIb -> STATE_EVEX
    5, // EvexVinsertf32x8VpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVinsertf64x4VpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVinserti32x4VdqHdqWdqIb -> STATE_EVEX
    5, // EvexVinserti64x2VdqHdqWdqIb -> STATE_EVEX
    5, // EvexVinserti32x4VdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVinserti64x2VdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVinserti32x8VdqHdqWdqIb -> STATE_EVEX
    5, // EvexVinserti64x4VdqHdqWdqIb -> STATE_EVEX
    5, // EvexVinserti32x8VdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVinserti64x4VdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVextractf32x4WpsVpsIb -> STATE_EVEX
    5, // EvexVextractf64x2WpdVpdIb -> STATE_EVEX
    5, // EvexVextractf32x4WpsVpsIbKmask -> STATE_EVEX
    5, // EvexVextractf64x2WpdVpdIbKmask -> STATE_EVEX
    5, // EvexVextractf32x8WpsVpsIb -> STATE_EVEX
    5, // EvexVextractf64x4WpdVpdIb -> STATE_EVEX
    5, // EvexVextractf32x8WpsVpsIbKmask -> STATE_EVEX
    5, // EvexVextractf64x4WpdVpdIbKmask -> STATE_EVEX
    5, // EvexVextracti32x4WdqVdqIb -> STATE_EVEX
    5, // EvexVextracti64x2WdqVdqIb -> STATE_EVEX
    5, // EvexVextracti32x4WdqVdqIbKmask -> STATE_EVEX
    5, // EvexVextracti64x2WdqVdqIbKmask -> STATE_EVEX
    5, // EvexVextracti32x8WdqVdqIb -> STATE_EVEX
    5, // EvexVextracti64x4WdqVdqIb -> STATE_EVEX
    5, // EvexVextracti32x8WdqVdqIbKmask -> STATE_EVEX
    5, // EvexVextracti64x4WdqVdqIbKmask -> STATE_EVEX
    5, // EvexVbroadcastf32x2VpsWq -> STATE_EVEX
    5, // EvexVbroadcastf32x2VpsWqKmask -> STATE_EVEX
    5, // EvexVbroadcasti32x2VdqWq -> STATE_EVEX
    5, // EvexVbroadcasti32x2VdqWqKmask -> STATE_EVEX
    5, // EvexVbroadcastf32x4VpsWps -> STATE_EVEX
    5, // EvexVbroadcastf64x2VpdWpd -> STATE_EVEX
    5, // EvexVbroadcastf32x4VpsWpsKmask -> STATE_EVEX
    5, // EvexVbroadcastf64x2VpdWpdKmask -> STATE_EVEX
    5, // EvexVbroadcastf32x8VpsWps -> STATE_EVEX
    5, // EvexVbroadcastf64x4VpdWpd -> STATE_EVEX
    5, // EvexVbroadcastf32x8VpsWpsKmask -> STATE_EVEX
    5, // EvexVbroadcastf64x4VpdWpdKmask -> STATE_EVEX
    5, // EvexVbroadcasti32x4VdqWdq -> STATE_EVEX
    5, // EvexVbroadcasti64x2VdqWdq -> STATE_EVEX
    5, // EvexVbroadcasti32x4VdqWdqKmask -> STATE_EVEX
    5, // EvexVbroadcasti64x2VdqWdqKmask -> STATE_EVEX
    5, // EvexVbroadcasti32x8VdqWdq -> STATE_EVEX
    5, // EvexVbroadcasti64x4VdqWdq -> STATE_EVEX
    5, // EvexVbroadcasti32x8VdqWdqKmask -> STATE_EVEX
    5, // EvexVbroadcasti64x4VdqWdqKmask -> STATE_EVEX
    5, // EvexVpmulldVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmullqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmulldVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmullqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpadddVdqHdqWdq -> STATE_EVEX
    5, // EvexVpaddqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpadddVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpaddqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsubdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsubqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpanddVdqHdqWdq -> STATE_EVEX
    5, // EvexVpandqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpanddVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpandqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpandndVdqHdqWdq -> STATE_EVEX
    5, // EvexVpandnqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpandndVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpandnqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpordVdqHdqWdq -> STATE_EVEX
    5, // EvexVporqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpordVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVporqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpxordVdqHdqWdq -> STATE_EVEX
    5, // EvexVpxorqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpxordVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpxorqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVandpsVpsHpsWps -> STATE_EVEX
    5, // EvexVandpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVandpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVandpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVandnpsVpsHpsWps -> STATE_EVEX
    5, // EvexVandnpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVandnpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVandnpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVorpsVpsHpsWps -> STATE_EVEX
    5, // EvexVorpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVorpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVorpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVxorpsVpsHpsWps -> STATE_EVEX
    5, // EvexVxorpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVxorpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVxorpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVpmaxsdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxsqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxsdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxsqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxudVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxuqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpmaxudVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpmaxuqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminsdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminsqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminsdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminsqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminudVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminuqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpminudVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpminuqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexValigndVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexValignqVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpalignrVdqHdqWdqIb -> STATE_EVEX
    5, // EvexVpalignrVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVdbpsadbwVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpsrlvwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrlvdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrlvqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsravwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsravdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsravqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsllvwVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsllvdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsllvqVdqHdqWdq -> STATE_EVEX
    5, // EvexVprolvdVdqHdqWdq -> STATE_EVEX
    5, // EvexVprolvqVdqHdqWdq -> STATE_EVEX
    5, // EvexVprorvdVdqHdqWdq -> STATE_EVEX
    5, // EvexVprorvqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsrlvwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrlvdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsrlvqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsravwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsravdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsravqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsllvwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsllvdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsllvqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVprolvdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVprolvqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVprorvdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVprorvqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsradVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsraqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpsradVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsraqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpsradUdqIb -> STATE_EVEX
    5, // EvexVpsraqUdqIb -> STATE_EVEX
    5, // EvexVprordUdqIb -> STATE_EVEX
    5, // EvexVprorqUdqIb -> STATE_EVEX
    5, // EvexVproldUdqIb -> STATE_EVEX
    5, // EvexVprolqUdqIb -> STATE_EVEX
    5, // EvexVpsradUdqIbKmask -> STATE_EVEX
    5, // EvexVpsraqUdqIbKmask -> STATE_EVEX
    5, // EvexVprordUdqIbKmask -> STATE_EVEX
    5, // EvexVprorqUdqIbKmask -> STATE_EVEX
    5, // EvexVproldUdqIbKmask -> STATE_EVEX
    5, // EvexVprolqUdqIbKmask -> STATE_EVEX
    5, // EvexVmovdqu8VdqWdq -> STATE_EVEX
    5, // EvexVmovdqu16VdqWdq -> STATE_EVEX
    5, // EvexVmovdqu8VdqWdqKmask -> STATE_EVEX
    5, // EvexVmovdqu16VdqWdqKmask -> STATE_EVEX
    5, // EvexVmovdqu8WdqVdq -> STATE_EVEX
    5, // EvexVmovdqu16WdqVdq -> STATE_EVEX
    5, // EvexVmovdqu8WdqVdqKmask -> STATE_EVEX
    5, // EvexVmovdqu16WdqVdqKmask -> STATE_EVEX
    5, // EvexVmovdqu32VdqWdq -> STATE_EVEX
    5, // EvexVmovdqu64VdqWdq -> STATE_EVEX
    5, // EvexVmovdqu32VdqWdqKmask -> STATE_EVEX
    5, // EvexVmovdqu64VdqWdqKmask -> STATE_EVEX
    5, // EvexVmovdqu32WdqVdq -> STATE_EVEX
    5, // EvexVmovdqu64WdqVdq -> STATE_EVEX
    5, // EvexVmovdqu32WdqVdqKmask -> STATE_EVEX
    5, // EvexVmovdqu64WdqVdqKmask -> STATE_EVEX
    5, // EvexVmovdqa32VdqWdq -> STATE_EVEX
    5, // EvexVmovdqa64VdqWdq -> STATE_EVEX
    5, // EvexVmovdqa32VdqWdqKmask -> STATE_EVEX
    5, // EvexVmovdqa64VdqWdqKmask -> STATE_EVEX
    5, // EvexVmovdqa32WdqVdq -> STATE_EVEX
    5, // EvexVmovdqa64WdqVdq -> STATE_EVEX
    5, // EvexVmovdqa32WdqVdqKmask -> STATE_EVEX
    5, // EvexVmovdqa64WdqVdqKmask -> STATE_EVEX
    5, // EvexVrangepsVpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVrangepdVpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVrangessVssHpsWssIbKmask -> STATE_EVEX
    5, // EvexVrangesdVsdHpdWsdIbKmask -> STATE_EVEX
    5, // EvexVgetexppsVpsWps -> STATE_EVEX
    5, // EvexVgetexppdVpdWpd -> STATE_EVEX
    5, // EvexVgetexpssVssHpsWss -> STATE_EVEX
    5, // EvexVgetexpsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVgetexppsVpsWpsKmask -> STATE_EVEX
    5, // EvexVgetexppdVpdWpdKmask -> STATE_EVEX
    5, // EvexVgetexpssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVgetexpsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVgetmantpsVpsWpsIbKmask -> STATE_EVEX
    5, // EvexVgetmantpdVpdWpdIbKmask -> STATE_EVEX
    5, // EvexVgetmantssVssHpsWssIbKmask -> STATE_EVEX
    5, // EvexVgetmantsdVsdHpdWsdIbKmask -> STATE_EVEX
    5, // EvexVscalefpsVpsHpsWps -> STATE_EVEX
    5, // EvexVscalefpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVscalefssVssHpsWss -> STATE_EVEX
    5, // EvexVscalefsdVsdHpdWsd -> STATE_EVEX
    5, // EvexVscalefpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVscalefpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVscalefssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVscalefsdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVrcp14psVpsWpsKmask -> STATE_EVEX
    5, // EvexVrcp14pdVpdWpdKmask -> STATE_EVEX
    5, // EvexVrcp14ssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVrcp14sdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVrsqrt14psVpsWpsKmask -> STATE_EVEX
    5, // EvexVrsqrt14pdVpdWpdKmask -> STATE_EVEX
    5, // EvexVrsqrt14ssVssHpsWssKmask -> STATE_EVEX
    5, // EvexVrsqrt14sdVsdHpdWsdKmask -> STATE_EVEX
    5, // EvexVcvtps2uqqVdqWps -> STATE_EVEX
    5, // EvexVcvtpd2uqqVdqWpd -> STATE_EVEX
    5, // EvexVcvtps2uqqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvtpd2uqqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttps2uqqVdqWps -> STATE_EVEX
    5, // EvexVcvttps2uqqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2uqqVdqWpd -> STATE_EVEX
    5, // EvexVcvttpd2uqqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvtps2qqVdqWps -> STATE_EVEX
    5, // EvexVcvtps2qqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvtpd2qqVdqWpd -> STATE_EVEX
    5, // EvexVcvtpd2qqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttps2qqVdqWps -> STATE_EVEX
    5, // EvexVcvttps2qqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2qqVdqWpd -> STATE_EVEX
    5, // EvexVcvttpd2qqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttps2udqVdqWps -> STATE_EVEX
    5, // EvexVcvttpd2udqVdqWpd -> STATE_EVEX
    5, // EvexVcvttps2udqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2udqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvtps2udqVdqWps -> STATE_EVEX
    5, // EvexVcvtpd2udqVdqWpd -> STATE_EVEX
    5, // EvexVcvtps2udqVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvtpd2udqVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvtudq2pdVpdWdq -> STATE_EVEX
    5, // EvexVcvtudq2pdVpdWdqKmask -> STATE_EVEX
    5, // EvexVcvtuqq2pdVpdWdq -> STATE_EVEX
    5, // EvexVcvtuqq2pdVpdWdqKmask -> STATE_EVEX
    5, // EvexVcvtudq2psVpsWdq -> STATE_EVEX
    5, // EvexVcvtudq2psVpsWdqKmask -> STATE_EVEX
    5, // EvexVcvtuqq2psVpsWdq -> STATE_EVEX
    5, // EvexVcvtuqq2psVpsWdqKmask -> STATE_EVEX
    5, // EvexVcvtdq2pdVpdWdq -> STATE_EVEX
    5, // EvexVcvtdq2pdVpdWdqKmask -> STATE_EVEX
    5, // EvexVcvtqq2pdVpdWdq -> STATE_EVEX
    5, // EvexVcvtqq2pdVpdWdqKmask -> STATE_EVEX
    5, // EvexVcvtdq2psVpsWdq -> STATE_EVEX
    5, // EvexVcvtdq2psVpsWdqKmask -> STATE_EVEX
    5, // EvexVcvtqq2psVpsWdq -> STATE_EVEX
    5, // EvexVcvtqq2psVpsWdqKmask -> STATE_EVEX
    5, // EvexVfmadd132psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmadd132pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmadd213psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmadd213pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmadd231psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmadd231pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmadd132psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmadd132pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmadd213psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmadd213pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmadd231psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmadd231pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmadd132ssVpsHssWss -> STATE_EVEX
    5, // EvexVfmadd132sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfmadd213ssVpsHssWss -> STATE_EVEX
    5, // EvexVfmadd213sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfmadd231ssVpsHssWss -> STATE_EVEX
    5, // EvexVfmadd231sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfmadd132ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfmadd132sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfmadd213ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfmadd213sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfmadd231ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfmadd231sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfmaddsub132psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmaddsub132pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmaddsub213psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmaddsub213pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmaddsub231psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmaddsub231pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmaddsub132psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmaddsub132pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmaddsub213psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmaddsub213pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmaddsub231psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmaddsub231pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsubadd132psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmsubadd132pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmsubadd213psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmsubadd213pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmsubadd231psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmsubadd231pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmsubadd132psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmsubadd132pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsubadd213psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmsubadd213pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsubadd231psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmsubadd231pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsub132psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmsub132pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmsub213psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmsub213pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmsub231psVpsHpsWps -> STATE_EVEX
    5, // EvexVfmsub231pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfmsub132psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmsub132pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsub213psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmsub213pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsub231psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfmsub231pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfmsub132ssVpsHssWss -> STATE_EVEX
    5, // EvexVfmsub132sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfmsub213ssVpsHssWss -> STATE_EVEX
    5, // EvexVfmsub213sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfmsub231ssVpsHssWss -> STATE_EVEX
    5, // EvexVfmsub231sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfmsub132ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfmsub132sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfmsub213ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfmsub213sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfmsub231ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfmsub231sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfnmadd132psVpsHpsWps -> STATE_EVEX
    5, // EvexVfnmadd132pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfnmadd213psVpsHpsWps -> STATE_EVEX
    5, // EvexVfnmadd213pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfnmadd231psVpsHpsWps -> STATE_EVEX
    5, // EvexVfnmadd231pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfnmadd132psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfnmadd132pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfnmadd213psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfnmadd213pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfnmadd231psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfnmadd231pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfnmadd132ssVpsHssWss -> STATE_EVEX
    5, // EvexVfnmadd132sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfnmadd213ssVpsHssWss -> STATE_EVEX
    5, // EvexVfnmadd213sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfnmadd231ssVpsHssWss -> STATE_EVEX
    5, // EvexVfnmadd231sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfnmadd132ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfnmadd132sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfnmadd213ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfnmadd213sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfnmadd231ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfnmadd231sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfnmsub132psVpsHpsWps -> STATE_EVEX
    5, // EvexVfnmsub132pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfnmsub213psVpsHpsWps -> STATE_EVEX
    5, // EvexVfnmsub213pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfnmsub231psVpsHpsWps -> STATE_EVEX
    5, // EvexVfnmsub231pdVpdHpdWpd -> STATE_EVEX
    5, // EvexVfnmsub132psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfnmsub132pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfnmsub213psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfnmsub213pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfnmsub231psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVfnmsub231pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVfnmsub132ssVpsHssWss -> STATE_EVEX
    5, // EvexVfnmsub132sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfnmsub213ssVpsHssWss -> STATE_EVEX
    5, // EvexVfnmsub213sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfnmsub231ssVpsHssWss -> STATE_EVEX
    5, // EvexVfnmsub231sdVpdHsdWsd -> STATE_EVEX
    5, // EvexVfnmsub132ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfnmsub132sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfnmsub213ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfnmsub213sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVfnmsub231ssVpsHssWssKmask -> STATE_EVEX
    5, // EvexVfnmsub231sdVpdHsdWsdKmask -> STATE_EVEX
    5, // EvexVpcmpbKgqHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpwKgdHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpubKgqHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpuwKgdHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpdKgwHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpqKgbHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpudKgwHdqWdqIb -> STATE_EVEX
    5, // EvexVpcmpuqKgbHdqWdqIb -> STATE_EVEX
    5, // EvexVptestmbKgqHdqWdq -> STATE_EVEX
    5, // EvexVptestmwKgdHdqWdq -> STATE_EVEX
    5, // EvexVptestnmbKgqHdqWdq -> STATE_EVEX
    5, // EvexVptestnmwKgdHdqWdq -> STATE_EVEX
    5, // EvexVptestmdKgwHdqWdq -> STATE_EVEX
    5, // EvexVptestmqKgbHdqWdq -> STATE_EVEX
    5, // EvexVptestnmdKgwHdqWdq -> STATE_EVEX
    5, // EvexVptestnmqKgbHdqWdq -> STATE_EVEX
    5, // EvexVpternlogdVdqHdqWdqIb -> STATE_EVEX
    5, // EvexVpternlogqVdqHdqWdqIb -> STATE_EVEX
    5, // EvexVpternlogdVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpternlogqVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVgatherdpsVpsVsib -> STATE_EVEX
    5, // EvexVgatherdpdVpdVsib -> STATE_EVEX
    5, // EvexVgatherqpsVpsVsib -> STATE_EVEX
    5, // EvexVgatherqpdVpdVsib -> STATE_EVEX
    5, // EvexVgatherddVdqVsib -> STATE_EVEX
    5, // EvexVgatherdqVdqVsib -> STATE_EVEX
    5, // EvexVgatherqdVdqVsib -> STATE_EVEX
    5, // EvexVgatherqqVdqVsib -> STATE_EVEX
    5, // EvexVscatterdpsVsibVps -> STATE_EVEX
    5, // EvexVscatterdpdVsibVpd -> STATE_EVEX
    5, // EvexVscatterqpsVsibVps -> STATE_EVEX
    5, // EvexVscatterqpdVsibVpd -> STATE_EVEX
    5, // EvexVscatterddVsibVdq -> STATE_EVEX
    5, // EvexVscatterdqVsibVdq -> STATE_EVEX
    5, // EvexVscatterqdVsibVdq -> STATE_EVEX
    5, // EvexVscatterqqVsibVdq -> STATE_EVEX
    5, // EvexVblendmpsVpsHpsWps -> STATE_EVEX
    5, // EvexVblendmpdVpdHpdWpd -> STATE_EVEX
    5, // EvexVpblendmdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpblendmqVdqHdqWdq -> STATE_EVEX
    5, // EvexVpblendmbVdqHdqWdq -> STATE_EVEX
    5, // EvexVpblendmwVdqHdqWdq -> STATE_EVEX
    5, // EvexVshufi32x4VdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVshufi64x2VdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVshuff32x4VpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVshuff64x2VpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVexpandpsVpsWps -> STATE_EVEX
    5, // EvexVexpandpdVpdWpd -> STATE_EVEX
    5, // EvexVexpandpsVpsWpsKmask -> STATE_EVEX
    5, // EvexVexpandpdVpdWpdKmask -> STATE_EVEX
    5, // EvexVcompresspsWpsVps -> STATE_EVEX
    5, // EvexVcompresspdWpdVpd -> STATE_EVEX
    5, // EvexVcompresspsWpsVpsKmask -> STATE_EVEX
    5, // EvexVcompresspdWpdVpdKmask -> STATE_EVEX
    5, // EvexVpexpandbVdqWdq -> STATE_EVEX
    5, // EvexVpexpandwVdqWdq -> STATE_EVEX
    5, // EvexVpexpandbVdqWdqKmask -> STATE_EVEX
    5, // EvexVpexpandwVdqWdqKmask -> STATE_EVEX
    5, // EvexVpexpanddVdqWdq -> STATE_EVEX
    5, // EvexVpexpandqVdqWdq -> STATE_EVEX
    5, // EvexVpexpanddVdqWdqKmask -> STATE_EVEX
    5, // EvexVpexpandqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpcompressbWdqVdq -> STATE_EVEX
    5, // EvexVpcompresswWdqVdq -> STATE_EVEX
    5, // EvexVpcompressbWdqVdqKmask -> STATE_EVEX
    5, // EvexVpcompresswWdqVdqKmask -> STATE_EVEX
    5, // EvexVpcompressdWdqVdq -> STATE_EVEX
    5, // EvexVpcompressqWdqVdq -> STATE_EVEX
    5, // EvexVpcompressdWdqVdqKmask -> STATE_EVEX
    5, // EvexVpcompressqWdqVdqKmask -> STATE_EVEX
    5, // EvexVfixupimmssVssHssWssIbKmask -> STATE_EVEX
    5, // EvexVfixupimmsdVsdHsdWsdIbKmask -> STATE_EVEX
    5, // EvexVfixupimmpsVpsHpsWpsIb -> STATE_EVEX
    5, // EvexVfixupimmpdVpdHpdWpdIb -> STATE_EVEX
    5, // EvexVfixupimmpsVpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVfixupimmpdVpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVfpclasspsKgwWpsIbKmask -> STATE_EVEX
    5, // EvexVfpclasspdKgbWpdIbKmask -> STATE_EVEX
    5, // EvexVfpclassssKgbWssIbKmask -> STATE_EVEX
    5, // EvexVfpclasssdKgbWsdIbKmask -> STATE_EVEX
    5, // EvexVreducepsVpsWpsIbKmask -> STATE_EVEX
    5, // EvexVreducepdVpdWpdIbKmask -> STATE_EVEX
    5, // EvexVreducessVssHpsWssIbKmask -> STATE_EVEX
    5, // EvexVreducesdVsdHpdWsdIbKmask -> STATE_EVEX
    5, // EvexVpermt2dVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermt2qVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermi2dVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermi2qVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermt2psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVpermt2pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVpermi2psVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVpermi2pdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVpermdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpermpsVpsHpsWpsKmask -> STATE_EVEX
    5, // EvexVpermpdVpdHpdWpdKmask -> STATE_EVEX
    5, // EvexVpconflictdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpconflictqVdqWdqKmask -> STATE_EVEX
    5, // EvexVplzcntdVdqWdqKmask -> STATE_EVEX
    5, // EvexVplzcntqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpmovm2bVdqKeq -> STATE_EVEX
    5, // EvexVpmovm2wVdqKed -> STATE_EVEX
    5, // EvexVpmovm2dVdqKew -> STATE_EVEX
    5, // EvexVpmovm2qVdqKeb -> STATE_EVEX
    5, // EvexVpmovb2mKgqWdq -> STATE_EVEX
    5, // EvexVpmovw2mKgdWdq -> STATE_EVEX
    5, // EvexVpmovd2mKgwWdq -> STATE_EVEX
    5, // EvexVpmovq2mKgbWdq -> STATE_EVEX
    5, // EvexVpopcntbVdqWdqKmask -> STATE_EVEX
    5, // EvexVpopcntwVdqWdqKmask -> STATE_EVEX
    5, // EvexVpopcntdVdqWdqKmask -> STATE_EVEX
    5, // EvexVpopcntqVdqWdqKmask -> STATE_EVEX
    5, // EvexVpshrddVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshrdqVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshrdvdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpshrdvqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpshlddVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshldqVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshldvdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpshldvqVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVcvtss2siGdWss -> STATE_EVEX
    5, // EvexVcvtss2siGqWss -> STATE_EVEX
    5, // EvexVcvtsd2siGdWsd -> STATE_EVEX
    5, // EvexVcvtsd2siGqWsd -> STATE_EVEX
    5, // EvexVcvttss2siGdWss -> STATE_EVEX
    5, // EvexVcvttss2siGqWss -> STATE_EVEX
    5, // EvexVcvttsd2siGdWsd -> STATE_EVEX
    5, // EvexVcvttsd2siGqWsd -> STATE_EVEX
    5, // EvexVmovdVdqEd -> STATE_EVEX
    5, // EvexVmovqVdqEq -> STATE_EVEX
    5, // EvexVmovdEdVd -> STATE_EVEX
    5, // EvexVmovqEqVq -> STATE_EVEX
    5, // EvexVcvtsi2ssVssEd -> STATE_EVEX
    5, // EvexVcvtsi2ssVssEq -> STATE_EVEX
    5, // EvexVcvtsi2sdVsdEd -> STATE_EVEX
    5, // EvexVcvtsi2sdVsdEq -> STATE_EVEX
    5, // EvexVcvtusi2ssVssEd -> STATE_EVEX
    5, // EvexVcvtusi2ssVssEq -> STATE_EVEX
    5, // EvexVcvtusi2sdVsdEd -> STATE_EVEX
    5, // EvexVcvtusi2sdVsdEq -> STATE_EVEX
    5, // EvexVcvtss2usiGdWss -> STATE_EVEX
    5, // EvexVcvtss2usiGqWss -> STATE_EVEX
    5, // EvexVcvtsd2usiGdWsd -> STATE_EVEX
    5, // EvexVcvtsd2usiGqWsd -> STATE_EVEX
    5, // EvexVcvttss2usiGdWss -> STATE_EVEX
    5, // EvexVcvttss2usiGqWss -> STATE_EVEX
    5, // EvexVcvttsd2usiGdWsd -> STATE_EVEX
    5, // EvexVcvttsd2usiGqWsd -> STATE_EVEX
    5, // EvexVpinsrbVdqEbIb -> STATE_EVEX
    5, // EvexVpinsrwVdqEwIb -> STATE_EVEX
    5, // EvexVpextrwGdUdqIb -> STATE_EVEX
    5, // EvexVpextrbEdVdqIbR -> STATE_EVEX
    5, // EvexVpextrbMbVdqIbM -> STATE_EVEX
    5, // EvexVpextrwEdVdqIbR -> STATE_EVEX
    5, // EvexVpextrwMwVdqIbM -> STATE_EVEX
    5, // EvexVpinsrdVdqEdIb -> STATE_EVEX
    5, // EvexVpinsrqVdqEqIb -> STATE_EVEX
    5, // EvexVpextrdEdVdqIb -> STATE_EVEX
    5, // EvexVpextrqEqVdqIb -> STATE_EVEX
    5, // EvexVpbroadcastmb2qVdqKeb -> STATE_EVEX
    5, // EvexVpbroadcastmw2dVdqKew -> STATE_EVEX
    5, // EvexVpdpbusdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbusdsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwssdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwssdsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbusdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpbusdsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwssdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwssdsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpshufbitqmbKgqHdqWdqKmask -> STATE_EVEX
    5, // EvexVp2intersectdKgqHdqWdq -> STATE_EVEX
    5, // EvexVp2intersectqKgqHdqWdq -> STATE_EVEX
    5, // EvexVpshrdwVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshrdvwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpshldwVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVpshldvwVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVaddshVshHphWsh -> STATE_EVEX
    5, // EvexVaddshVshHphWshKmask -> STATE_EVEX
    5, // EvexVsubshVshHphWsh -> STATE_EVEX
    5, // EvexVsubshVshHphWshKmask -> STATE_EVEX
    5, // EvexVmulshVshHphWsh -> STATE_EVEX
    5, // EvexVmulshVshHphWshKmask -> STATE_EVEX
    5, // EvexVdivshVshHphWsh -> STATE_EVEX
    5, // EvexVdivshVshHphWshKmask -> STATE_EVEX
    5, // EvexVminshVshHphWsh -> STATE_EVEX
    5, // EvexVminshVshHphWshKmask -> STATE_EVEX
    5, // EvexVmaxshVshHphWsh -> STATE_EVEX
    5, // EvexVmaxshVshHphWshKmask -> STATE_EVEX
    5, // EvexVscalefshVshHphWsh -> STATE_EVEX
    5, // EvexVscalefshVshHphWshKmask -> STATE_EVEX
    5, // EvexVaddphVphHphWph -> STATE_EVEX
    5, // EvexVaddphVphHphWphKmask -> STATE_EVEX
    5, // EvexVsubphVphHphWph -> STATE_EVEX
    5, // EvexVsubphVphHphWphKmask -> STATE_EVEX
    5, // EvexVmulphVphHphWph -> STATE_EVEX
    5, // EvexVmulphVphHphWphKmask -> STATE_EVEX
    5, // EvexVdivphVphHphWph -> STATE_EVEX
    5, // EvexVdivphVphHphWphKmask -> STATE_EVEX
    5, // EvexVminphVphHphWph -> STATE_EVEX
    5, // EvexVminphVphHphWphKmask -> STATE_EVEX
    5, // EvexVmaxphVphHphWph -> STATE_EVEX
    5, // EvexVmaxphVphHphWphKmask -> STATE_EVEX
    5, // EvexVscalefphVphHphWph -> STATE_EVEX
    5, // EvexVscalefphVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmadd132shVphHshWsh -> STATE_EVEX
    5, // EvexVfmadd132shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfmadd213shVphHshWsh -> STATE_EVEX
    5, // EvexVfmadd213shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfmadd231shVphHshWsh -> STATE_EVEX
    5, // EvexVfmadd231shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfnmadd132shVphHshWsh -> STATE_EVEX
    5, // EvexVfnmadd132shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfnmadd213shVphHshWsh -> STATE_EVEX
    5, // EvexVfnmadd213shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfnmadd231shVphHshWsh -> STATE_EVEX
    5, // EvexVfnmadd231shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfmsub132shVphHshWsh -> STATE_EVEX
    5, // EvexVfmsub132shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfmsub213shVphHshWsh -> STATE_EVEX
    5, // EvexVfmsub213shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfmsub231shVphHshWsh -> STATE_EVEX
    5, // EvexVfmsub231shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfnmsub132shVphHshWsh -> STATE_EVEX
    5, // EvexVfnmsub132shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfnmsub213shVphHshWsh -> STATE_EVEX
    5, // EvexVfnmsub213shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfnmsub231shVphHshWsh -> STATE_EVEX
    5, // EvexVfnmsub231shVphHshWshKmask -> STATE_EVEX
    5, // EvexVfmadd132phVphHphWph -> STATE_EVEX
    5, // EvexVfmadd132phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmadd213phVphHphWph -> STATE_EVEX
    5, // EvexVfmadd213phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmadd231phVphHphWph -> STATE_EVEX
    5, // EvexVfmadd231phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmadd132phVphHphWph -> STATE_EVEX
    5, // EvexVfnmadd132phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmadd213phVphHphWph -> STATE_EVEX
    5, // EvexVfnmadd213phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmadd231phVphHphWph -> STATE_EVEX
    5, // EvexVfnmadd231phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsub132phVphHphWph -> STATE_EVEX
    5, // EvexVfmsub132phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsub213phVphHphWph -> STATE_EVEX
    5, // EvexVfmsub213phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsub231phVphHphWph -> STATE_EVEX
    5, // EvexVfmsub231phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmsub132phVphHphWph -> STATE_EVEX
    5, // EvexVfnmsub132phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmsub213phVphHphWph -> STATE_EVEX
    5, // EvexVfnmsub213phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmsub231phVphHphWph -> STATE_EVEX
    5, // EvexVfnmsub231phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmaddsub132phVphHphWph -> STATE_EVEX
    5, // EvexVfmaddsub132phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmaddsub213phVphHphWph -> STATE_EVEX
    5, // EvexVfmaddsub213phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmaddsub231phVphHphWph -> STATE_EVEX
    5, // EvexVfmaddsub231phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsubadd132phVphHphWph -> STATE_EVEX
    5, // EvexVfmsubadd132phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsubadd213phVphHphWph -> STATE_EVEX
    5, // EvexVfmsubadd213phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsubadd231phVphHphWph -> STATE_EVEX
    5, // EvexVfmsubadd231phVphHphWphKmask -> STATE_EVEX
    5, // EvexVfpclassphKgdWphIbKmask -> STATE_EVEX
    5, // EvexVfpclassshKgbWshIbKmask -> STATE_EVEX
    5, // EvexVucomishVshWsh -> STATE_EVEX
    5, // EvexVcomishVshWsh -> STATE_EVEX
    5, // EvexVcmpphKgdHphWphIb -> STATE_EVEX
    5, // EvexVcmpshKgbHshWshIb -> STATE_EVEX
    5, // EvexVsqrtphVphWph -> STATE_EVEX
    5, // EvexVsqrtphVphWphKmask -> STATE_EVEX
    5, // EvexVsqrtshVshHphWsh -> STATE_EVEX
    5, // EvexVsqrtshVshHphWshKmask -> STATE_EVEX
    5, // EvexVgetexpphVphWph -> STATE_EVEX
    5, // EvexVgetexpphVphWphKmask -> STATE_EVEX
    5, // EvexVgetexpshVshHphWsh -> STATE_EVEX
    5, // EvexVgetexpshVshHphWshKmask -> STATE_EVEX
    5, // EvexVmovshVshWsh -> STATE_EVEX
    5, // EvexVmovshWshVsh -> STATE_EVEX
    5, // EvexVmovshVshWshKmask -> STATE_EVEX
    5, // EvexVmovshWshVshKmask -> STATE_EVEX
    5, // EvexVmovshVshHphWsh -> STATE_EVEX
    5, // EvexVmovshWshHphVsh -> STATE_EVEX
    5, // EvexVmovshVshHphWshKmask -> STATE_EVEX
    5, // EvexVmovshWshHphVshKmask -> STATE_EVEX
    5, // EvexVmovwVshEw -> STATE_EVEX
    5, // EvexVmovwEdVsh -> STATE_EVEX
    5, // EvexVcvtph2uwVdqWps -> STATE_EVEX
    5, // EvexVcvtph2uwVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvtph2wVdqWps -> STATE_EVEX
    5, // EvexVcvtph2wVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttph2uwVdqWps -> STATE_EVEX
    5, // EvexVcvttph2uwVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttph2wVdqWps -> STATE_EVEX
    5, // EvexVcvttph2wVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvtuw2phVphWdq -> STATE_EVEX
    5, // EvexVcvtuw2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtw2phVphWdq -> STATE_EVEX
    5, // EvexVcvtw2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtph2psxVpsWph -> STATE_EVEX
    5, // EvexVcvtph2psxVpsWphKmask -> STATE_EVEX
    5, // EvexVcvtph2dqVdqWph -> STATE_EVEX
    5, // EvexVcvtph2dqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvtph2udqVdqWph -> STATE_EVEX
    5, // EvexVcvtph2udqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvttph2dqVdqWph -> STATE_EVEX
    5, // EvexVcvttph2dqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvttph2udqVdqWph -> STATE_EVEX
    5, // EvexVcvttph2udqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvtph2pdVpdWph -> STATE_EVEX
    5, // EvexVcvtph2pdVpdWphKmask -> STATE_EVEX
    5, // EvexVcvtph2qqVdqWph -> STATE_EVEX
    5, // EvexVcvtph2qqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvtph2uqqVdqWph -> STATE_EVEX
    5, // EvexVcvtph2uqqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvttph2qqVdqWph -> STATE_EVEX
    5, // EvexVcvttph2qqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvttph2uqqVdqWph -> STATE_EVEX
    5, // EvexVcvttph2uqqVdqWphKmask -> STATE_EVEX
    5, // EvexVcvtps2phxVphWdq -> STATE_EVEX
    5, // EvexVcvtps2phxVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtdq2phVphWdq -> STATE_EVEX
    5, // EvexVcvtdq2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtudq2phVphWdq -> STATE_EVEX
    5, // EvexVcvtudq2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtpd2phVphWdq -> STATE_EVEX
    5, // EvexVcvtpd2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtqq2phVphWdq -> STATE_EVEX
    5, // EvexVcvtqq2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtuqq2phVphWdq -> STATE_EVEX
    5, // EvexVcvtuqq2phVphWdqKmask -> STATE_EVEX
    5, // EvexVcvtsh2ssVssWsh -> STATE_EVEX
    5, // EvexVcvtsh2ssVssWshKmask -> STATE_EVEX
    5, // EvexVcvtsh2sdVsdWsh -> STATE_EVEX
    5, // EvexVcvtsh2sdVsdWshKmask -> STATE_EVEX
    5, // EvexVcvtss2shVssWsh -> STATE_EVEX
    5, // EvexVcvtss2shVssWshKmask -> STATE_EVEX
    5, // EvexVcvtsd2shVssWsh -> STATE_EVEX
    5, // EvexVcvtsd2shVssWshKmask -> STATE_EVEX
    5, // EvexVcvtsh2siGdWss -> STATE_EVEX
    5, // EvexVcvtsh2siGqWss -> STATE_EVEX
    5, // EvexVcvtsh2usiGdWss -> STATE_EVEX
    5, // EvexVcvtsh2usiGqWss -> STATE_EVEX
    5, // EvexVcvttsh2siGdWss -> STATE_EVEX
    5, // EvexVcvttsh2siGqWss -> STATE_EVEX
    5, // EvexVcvttsh2usiGdWss -> STATE_EVEX
    5, // EvexVcvttsh2usiGqWss -> STATE_EVEX
    5, // EvexVcvtsi2shVshEd -> STATE_EVEX
    5, // EvexVcvtsi2shVshEq -> STATE_EVEX
    5, // EvexVcvtusi2shVshEd -> STATE_EVEX
    5, // EvexVcvtusi2shVshEq -> STATE_EVEX
    5, // EvexVgetmantphVphWphIbKmask -> STATE_EVEX
    5, // EvexVgetmantshVshHphWshIbKmask -> STATE_EVEX
    5, // EvexVreducephVphWphIbKmask -> STATE_EVEX
    5, // EvexVreduceshVshHphWshIbKmask -> STATE_EVEX
    5, // EvexVrndscalephVphWphIbKmask -> STATE_EVEX
    5, // EvexVrndscaleshVshHphWshIbKmask -> STATE_EVEX
    5, // EvexVrcpphVphWphKmask -> STATE_EVEX
    5, // EvexVrcpshVshHphWshKmask -> STATE_EVEX
    5, // EvexVrsqrtphVphWphKmask -> STATE_EVEX
    5, // EvexVrsqrtshVshHphWshKmask -> STATE_EVEX
    5, // EvexVfmulcshVshHphWshKmask -> STATE_EVEX
    5, // EvexVfcmulcshVshHphWshKmask -> STATE_EVEX
    5, // EvexVfmulcphVphHphWphKmask -> STATE_EVEX
    5, // EvexVfcmulcphVphHphWphKmask -> STATE_EVEX
    5, // EvexVfmaddcshVshHphWshKmask -> STATE_EVEX
    5, // EvexVfcmaddcshVshHphWshKmask -> STATE_EVEX
    5, // EvexVfmaddcphVphHphWphKmask -> STATE_EVEX
    5, // EvexVfcmaddcphVphHphWphKmask -> STATE_EVEX
    5, // EvexVaesencVdqHdqWdq -> STATE_EVEX
    5, // EvexVaesenclastVdqHdqWdq -> STATE_EVEX
    5, // EvexVaesdecVdqHdqWdq -> STATE_EVEX
    5, // EvexVaesdeclastVdqHdqWdq -> STATE_EVEX
    5, // EvexVpclmulqdqVdqHdqWdqIb -> STATE_EVEX
    5, // EvexVgf2p8affineqbVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVgf2p8affineinvqbVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVgf2p8mulbVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVsm4key4VdqHdqWdq -> STATE_EVEX
    5, // EvexVsm4rnds4VdqHdqWdq -> STATE_EVEX
    5, // EvexVucomxssVssWss -> STATE_EVEX
    5, // EvexVcomxssVssWss -> STATE_EVEX
    5, // EvexVucomxsdVsdWsd -> STATE_EVEX
    5, // EvexVcomxsdVsdWsd -> STATE_EVEX
    5, // EvexVucomxshVshWsh -> STATE_EVEX
    5, // EvexVcomxshVshWsh -> STATE_EVEX
    5, // EvexVpdpbssdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbssdsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbsudVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbsudsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbuudVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbuudsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpbssdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpbssdsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpbsudVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpbsudsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpbuudVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpbuudsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwsudVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwsudsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwusdVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwusdsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwuudVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwuudsVdqHdqWdq -> STATE_EVEX
    5, // EvexVpdpwsudVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwsudsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwusdVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwusdsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwuudVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVpdpwuudsVdqHdqWdqKmask -> STATE_EVEX
    5, // EvexVmpsadbwVdqHdqWdqIb -> STATE_EVEX
    5, // EvexVmpsadbwVdqHdqWdqIbKmask -> STATE_EVEX
    5, // EvexVdpphpsVpsHdqWdqKmask -> STATE_EVEX
    5, // EvexVaddbf16VphHphWph -> STATE_EVEX
    5, // EvexVaddbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVsubbf16VphHphWph -> STATE_EVEX
    5, // EvexVsubbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVdivbf16VphHphWph -> STATE_EVEX
    5, // EvexVdivbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVmulbf16VphHphWph -> STATE_EVEX
    5, // EvexVmulbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVminpbf16VphHphWph -> STATE_EVEX
    5, // EvexVminpbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVmaxpbf16VphHphWph -> STATE_EVEX
    5, // EvexVmaxpbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVscalefpbf16VphHphWph -> STATE_EVEX
    5, // EvexVscalefpbf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVsqrtbf16VphWph -> STATE_EVEX
    5, // EvexVsqrtbf16VphWphKmask -> STATE_EVEX
    5, // EvexVgetexppbf16VphWph -> STATE_EVEX
    5, // EvexVgetexppbf16VphWphKmask -> STATE_EVEX
    5, // EvexVfmadd132bf16VphHphWph -> STATE_EVEX
    5, // EvexVfmadd132bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfmadd213bf16VphHphWph -> STATE_EVEX
    5, // EvexVfmadd213bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfmadd231bf16VphHphWph -> STATE_EVEX
    5, // EvexVfmadd231bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsub132bf16VphHphWph -> STATE_EVEX
    5, // EvexVfmsub132bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsub213bf16VphHphWph -> STATE_EVEX
    5, // EvexVfmsub213bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfmsub231bf16VphHphWph -> STATE_EVEX
    5, // EvexVfmsub231bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmadd132bf16VphHphWph -> STATE_EVEX
    5, // EvexVfnmadd132bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmadd213bf16VphHphWph -> STATE_EVEX
    5, // EvexVfnmadd213bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmadd231bf16VphHphWph -> STATE_EVEX
    5, // EvexVfnmadd231bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmsub132bf16VphHphWph -> STATE_EVEX
    5, // EvexVfnmsub132bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmsub213bf16VphHphWph -> STATE_EVEX
    5, // EvexVfnmsub213bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfnmsub231bf16VphHphWph -> STATE_EVEX
    5, // EvexVfnmsub231bf16VphHphWphKmask -> STATE_EVEX
    5, // EvexVfpclasspbf16KgdWphIbKmask -> STATE_EVEX
    5, // EvexVcmppbf16KgdHphWphIb -> STATE_EVEX
    5, // EvexVcomisbf16VshWsh -> STATE_EVEX
    5, // EvexVgetmantpbf16VphWphIbKmask -> STATE_EVEX
    5, // EvexVreducebf16VphWphIbKmask -> STATE_EVEX
    5, // EvexVrndscalebf16VphWphIbKmask -> STATE_EVEX
    5, // EvexVrcppbf16VphWph -> STATE_EVEX
    5, // EvexVrcppbf16VphWphKmask -> STATE_EVEX
    5, // EvexVrsqrtpbf16VphWph -> STATE_EVEX
    5, // EvexVrsqrtpbf16VphWphKmask -> STATE_EVEX
    5, // EvexVminmaxpsVpsHpsWpsIbKmask -> STATE_EVEX
    5, // EvexVminmaxssVssHpsWssIbKmask -> STATE_EVEX
    5, // EvexVminmaxpdVpdHpdWpdIbKmask -> STATE_EVEX
    5, // EvexVminmaxsdVsdHpdWsdIbKmask -> STATE_EVEX
    5, // EvexVminmaxphVphHphWphIbKmask -> STATE_EVEX
    5, // EvexVminmaxshVshHphWshIbKmask -> STATE_EVEX
    5, // EvexVminmaxbf16VphHphWphIbKmask -> STATE_EVEX
    5, // EvexVcvt2ps2phxVphHpsWpsKmask -> STATE_EVEX
    5, // EvexVcvttps2qqsVdqWps -> STATE_EVEX
    5, // EvexVcvttps2qqsVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2qqsVdqWpd -> STATE_EVEX
    5, // EvexVcvttpd2qqsVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttps2uqqsVdqWps -> STATE_EVEX
    5, // EvexVcvttps2uqqsVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2uqqsVdqWpd -> STATE_EVEX
    5, // EvexVcvttpd2uqqsVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttps2dqsVdqWps -> STATE_EVEX
    5, // EvexVcvttps2dqsVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2dqsVdqWpd -> STATE_EVEX
    5, // EvexVcvttpd2dqsVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttps2udqsVdqWps -> STATE_EVEX
    5, // EvexVcvttpd2udqsVdqWpd -> STATE_EVEX
    5, // EvexVcvttps2udqsVdqWpsKmask -> STATE_EVEX
    5, // EvexVcvttpd2udqsVdqWpdKmask -> STATE_EVEX
    5, // EvexVcvttss2sisGdWss -> STATE_EVEX
    5, // EvexVcvttss2sisGqWss -> STATE_EVEX
    5, // EvexVcvttsd2sisGdWsd -> STATE_EVEX
    5, // EvexVcvttsd2sisGqWsd -> STATE_EVEX
    5, // EvexVcvttss2usisGdWss -> STATE_EVEX
    5, // EvexVcvttss2usisGqWss -> STATE_EVEX
    5, // EvexVcvttsd2usisGdWsd -> STATE_EVEX
    5, // EvexVcvttsd2usisGqWsd -> STATE_EVEX
    5, // EvexVmovwVshWsh -> STATE_EVEX
    5, // EvexVmovwWshVsh -> STATE_EVEX
    5, // EvexVmovdVdWd -> STATE_EVEX
    5, // EvexVmovdWdVd -> STATE_EVEX
    5, // EvexVcvthf82phVphWf8Kmask -> STATE_EVEX
    5, // EvexVcvtph2bf8Vf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtph2bf8sVf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvt2ph2bf8Vf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvt2ph2bf8sVf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtbiasph2bf8Vf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtbiasph2bf8sVf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtph2hf8Vf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtph2hf8sVf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvt2ph2hf8Vf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvt2ph2hf8sVf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtbiasph2hf8Vf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtbiasph2hf8sVf8hdqWphKmask -> STATE_EVEX
    5, // EvexVcvtbf162ibsV8bWph -> STATE_EVEX
    5, // EvexVcvtbf162ibsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvtbf162iubsV8bWph -> STATE_EVEX
    5, // EvexVcvtbf162iubsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvttbf162ibsV8bWph -> STATE_EVEX
    5, // EvexVcvttbf162ibsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvttbf162iubsV8bWph -> STATE_EVEX
    5, // EvexVcvttbf162iubsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvtph2ibsV8bWph -> STATE_EVEX
    5, // EvexVcvtph2ibsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvtph2iubsV8bWph -> STATE_EVEX
    5, // EvexVcvtph2iubsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvttph2ibsV8bWph -> STATE_EVEX
    5, // EvexVcvttph2ibsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvttph2iubsV8bWph -> STATE_EVEX
    5, // EvexVcvttph2iubsV8bWphKmask -> STATE_EVEX
    5, // EvexVcvtps2ibsV8bWps -> STATE_EVEX
    5, // EvexVcvtps2ibsV8bWpsKmask -> STATE_EVEX
    5, // EvexVcvtps2iubsV8bWps -> STATE_EVEX
    5, // EvexVcvtps2iubsV8bWpsKmask -> STATE_EVEX
    5, // EvexVcvttps2ibsV8bWps -> STATE_EVEX
    5, // EvexVcvttps2ibsV8bWpsKmask -> STATE_EVEX
    5, // EvexVcvttps2iubsV8bWps -> STATE_EVEX
    5, // EvexVcvttps2iubsV8bWpsKmask -> STATE_EVEX
    6, // EvexTilemovrowVdqTrmIb -> STATE_AMX
    6, // EvexTilemovrowVdqTrmBd -> STATE_AMX
    6, // EvexTcvtrowd2psVpsTrmIb -> STATE_AMX
    6, // EvexTcvtrowd2psVpsTrmBd -> STATE_AMX
    6, // EvexTcvtrowps2phlVphTrmIb -> STATE_AMX
    6, // EvexTcvtrowps2phlVphTrmBd -> STATE_AMX
    6, // EvexTcvtrowps2phhVphTrmIb -> STATE_AMX
    6, // EvexTcvtrowps2phhVphTrmBd -> STATE_AMX
    6, // EvexTcvtrowps2bf16lVphTrmIb -> STATE_AMX
    6, // EvexTcvtrowps2bf16lVphTrmBd -> STATE_AMX
    6, // EvexTcvtrowps2bf16hVphTrmIb -> STATE_AMX
    6, // EvexTcvtrowps2bf16hVphTrmBd -> STATE_AMX
    5, // EvexVmovrsbVdqWdq -> STATE_EVEX
    5, // EvexVmovrsbVdqWdqKmask -> STATE_EVEX
    5, // EvexVmovrswVdqWdq -> STATE_EVEX
    5, // EvexVmovrswVdqWdqKmask -> STATE_EVEX
    5, // EvexVmovrsdVdqWdq -> STATE_EVEX
    5, // EvexVmovrsdVdqWdqKmask -> STATE_EVEX
    5, // EvexVmovrsqVdqWdq -> STATE_EVEX
    5, // EvexVmovrsqVdqWdqKmask -> STATE_EVEX
    0, // NoAvxState -> STATE_NONE
    0, // NoEvexState -> STATE_NONE
];

/// CPU state `opcode` requires before it may execute.
#[inline]
pub const fn opcode_prepare_class(opcode: Opcode) -> u8 {
    OPCODE_PREPARE[opcode as usize]
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
