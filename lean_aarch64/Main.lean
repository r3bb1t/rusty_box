import Leanbox.Lockstep
open Leanbox

def main (args : List String) : IO UInt32 := do
  match args with
  | "edk2" :: rest =>
    let code ← readFile "/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd"
    let vars ← readFile "/usr/share/AAVMF/AAVMF_VARS.fd"
    let dtb ← readFile ((← IO.getEnv "EDK2_DTB").getD "oracle/ref/dtb.bin")
    run1 (boot code vars dtb) rest "Shell> " "SHELL"
  | ["bare", path] => bare path
  | "linux" :: rest =>
    let kernel ← readFile ((← IO.getEnv "LINUX_IMAGE").getD "/tmp/alp/Image")
    let initrd ← readFile ((← IO.getEnv "LINUX_INITRD").getD "/tmp/alp/initramfs-virt")
    let dtb ← readFile ((← IO.getEnv "LINUX_DTB").getD "oracle/ref/linux_dtb.bin")
    run1 (bootLinux kernel initrd dtb) rest "\x01\x02" "UNTIL"
  | _ => IO.eprintln "usage: leanbox edk2|linux check K MAX [until:TEXT|type:TEXT]... | detail FROM COUNT; leanbox bare TESTS"
  return 0
