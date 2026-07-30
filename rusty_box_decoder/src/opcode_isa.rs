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

/// `X86Feature as u16` required by each opcode (2900 of 3677 are gated).
pub static OPCODE_ISA: [u16; 3677] = [
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
    ISA_ALWAYS, // FstpSpecialSti
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
    ISA_ALWAYS, // Pfrcpit2PqQq
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
    ISA_ALWAYS, // EvexVcvtudq2pdVpdWdq
    ISA_ALWAYS, // EvexVcvtudq2pdVpdWdqKmask
    78, // EvexVcvtuqq2pdVpdWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtuqq2pdVpdWdqKmask -> X86Feature::IsaAvx512Dq
    77, // EvexVcvtudq2psVpsWdq -> X86Feature::IsaAvx512
    77, // EvexVcvtudq2psVpsWdqKmask -> X86Feature::IsaAvx512
    78, // EvexVcvtuqq2psVpsWdq -> X86Feature::IsaAvx512Dq
    78, // EvexVcvtuqq2psVpsWdqKmask -> X86Feature::IsaAvx512Dq
    ISA_ALWAYS, // EvexVcvtdq2pdVpdWdq
    ISA_ALWAYS, // EvexVcvtdq2pdVpdWdqKmask
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
    ISA_ALWAYS, // EvexVcvtsi2sdVsdEd
    77, // EvexVcvtsi2sdVsdEq -> X86Feature::IsaAvx512
    77, // EvexVcvtusi2ssVssEd -> X86Feature::IsaAvx512
    77, // EvexVcvtusi2ssVssEq -> X86Feature::IsaAvx512
    ISA_ALWAYS, // EvexVcvtusi2sdVsdEd
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
];

/// Feature required to execute `opcode`, or `ISA_ALWAYS` if ungated.
#[inline]
pub fn opcode_isa_feature(opcode: Opcode) -> u16 {
    OPCODE_ISA[opcode as usize]
}

/// Number of opcodes carrying a real feature gate. Asserted by tests so
/// that a silent regeneration drop is caught.
pub const GATED_OPCODE_COUNT: usize = 2900;

/// Number of `Opcode` variants the table was generated against. A
/// mismatch with the enum means the table needs regenerating.
pub const OPCODE_VARIANT_COUNT: usize = 3677;

#[allow(dead_code)]
fn _feature_type_is_used(f: X86Feature) -> u16 {
    // Keeps the X86Feature import meaningful: the table stores raw
    // discriminants of exactly this enum.
    f as u16
}
