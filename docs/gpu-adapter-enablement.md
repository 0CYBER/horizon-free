# GPU adapter enablement log

This file records host checks and per-process environment attempts for trying
to expose a physical GPU to the existing WGPU parity probes. No system packages,
drivers, ICD registrations, or persistent environment settings were changed.

## H0 — Host inventory — DONE

The summary table below is followed by the verbatim outputs for each required
H0 command. Exit statuses are shown where a command failed. The ROCm tool's
terminal styling escape bytes are shown escaped as `\u001b` in this Markdown.

| Command | Output |
|---|---|
| `uname -a` | `Linux 1chael 7.2.0-1-cachyos-bore-lto #1 SMP PREEMPT_DYNAMIC Thu, 20 Aug 2026 16:00:34 +0000 x86_64 GNU/Linux` |
| `cat /etc/os-release` | `NAME="CachyOS Linux"`<br>`PRETTY_NAME="CachyOS"`<br>`ID=cachyos`<br>`ID_LIKE=arch`<br>`BUILD_ID=rolling`<br>`ANSI_COLOR="38;2;23;147;209"`<br>`HOME_URL="https://cachyos.org/"`<br>`DOCUMENTATION_URL="https://wiki.cachyos.org/"`<br>`SUPPORT_URL="https://discuss.cachyos.org/"`<br>`BUG_REPORT_URL="https://github.com/cachyos"`<br>`PRIVACY_POLICY_URL="https://terms.archlinux.org/docs/privacy-policy/"`<br>`LOGO=cachyos` |
| `lspci -nn \| grep -iE 'vga\|3d\|display\|render'` | `03:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. [AMD/ATI] Rembrandt [Radeon 680M] [1002:1681] (rev 03)` |
| `ls -la /dev/dri/` | exit 2: `ls: cannot access '/dev/dri/': No such file or directory` |
| `ls -la /dev/dxg` | exit 2: `ls: cannot access '/dev/dxg': No such file or directory` |
| `cat /proc/driver/nvidia/version 2>&1` | exit 1: `cat: /proc/driver/nvidia/version: No such file or directory` |
| `nvidia-smi 2>&1 \| head -20` | `/usr/bin/bash: line 1: nvidia-smi: command not found` |
| `rocm-smi 2>&1 \| head -20` | `WARNING: AMD GPU device(s) is/are in a low-power state. Check power control/runtime_status`<br>`Exception caught: map::at`<br>ROCm SMI printed its device table; the exact output from the final H0 invocation follows below. |
| `which vulkaninfo glxinfo clinfo eglinfo` | `/usr/bin/vulkaninfo`<br>`/usr/bin/glxinfo`<br>`/opt/rocm/bin/clinfo`<br>`/usr/bin/eglinfo` |
| `ls /usr/share/vulkan/icd.d/ 2>&1` | `lvp_icd.json`<br>`radeon_icd.json` |
| `for f in /usr/share/vulkan/icd.d/*.json; do echo "$f"; cat "$f"; done` | `/usr/share/vulkan/icd.d/lvp_icd.json` contains `"library_path": "libvulkan_lvp.so"`; `/usr/share/vulkan/icd.d/radeon_icd.json` contains `"library_path": "libvulkan_radeon.so"`. Both use file format `1.0.1` and API version `1.4.354`. |
| `env \| grep -iE 'wsl\|vulkan\|mesa\|wgpu\|dxg'` | exit 1; no matching variables were set. |
| `ls -la /sys/class/drm/` | `card1` and `renderD128` are present under the AMD PCI device; `renderD128` resolves to `.../0000:03:00.0/drm/renderD128`. The directory also lists `card1-DP-1` through `card1-DP-6`, `card1-HDMI-A-1`, `card1-Writeback-1`, and `card1-eDP-1`. |
| `lsmod \| grep -E 'nvidia\|amdgpu\|i915\|nouveau'` | `amdgpu` is loaded, along with `amdxcp`, `gpu_sched`, DRM helpers, and display-related modules. No NVIDIA, Intel `i915`, or Nouveau driver line appeared. |
| `test -e /.dockerenv ...; cat /proc/1/cgroup` | `/.dockerenv absent`<br>`0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.chromium.Chromium-4599.scope` |
| `systemd-detect-virt 2>&1; id` | `container-other`<br>`uid=1000(chael) gid=1000(chael) groups=1000(chael),65534(nobody)` |
| `getent group video; getent group render` | `video:x:983:chael`<br>`render:x:987:chael` |

### Verbatim H0 command outputs

```text
$ uname -a
Linux 1chael 7.2.0-1-cachyos-bore-lto #1 SMP PREEMPT_DYNAMIC Thu, 20 Aug 2026 16:00:34 +0000 x86_64 GNU/Linux

$ cat /etc/os-release
NAME="CachyOS Linux"
PRETTY_NAME="CachyOS"
ID=cachyos
ID_LIKE=arch
BUILD_ID=rolling
ANSI_COLOR="38;2;23;147;209"
HOME_URL="https://cachyos.org/"
DOCUMENTATION_URL="https://wiki.cachyos.org/"
SUPPORT_URL="https://discuss.cachyos.org/"
BUG_REPORT_URL="https://github.com/cachyos"
PRIVACY_POLICY_URL="https://terms.archlinux.org/docs/privacy-policy/"
LOGO=cachyos

$ lspci -nn | grep -iE 'vga|3d|display|render'
03:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. [AMD/ATI] Rembrandt [Radeon 680M] [1002:1681] (rev 03)

$ ls -la /dev/dri/
ls: cannot access '/dev/dri/': No such file or directory

$ ls -la /dev/dxg 2>&1
ls: cannot access '/dev/dxg': No such file or directory

$ cat /proc/driver/nvidia/version 2>&1
cat: /proc/driver/nvidia/version: No such file or directory

$ nvidia-smi 2>&1 | head -20
/usr/bin/bash: line 1: nvidia-smi: command not found

$ rocm-smi 2>&1 | head -20
WARNING: AMD GPU device(s) is/are in a low-power state. Check power control/runtime_status

Exception caught: map::at


========================================= ROCm System Management Interface =========================================
=================================================== Concise Info ===================================================
Device  Node  IDs              Temp    Power     Partitions          SCLK  MCLK    Fan  Perf  PwrCap  VRAM%  GPU%  
\u001b[3m              (DID,     GUID)  (Edge)  (Socket)  (Mem, Compute, ID)                                                \u001b[0m
====================================================================================================================
0       1     0x1681,   35407  49.0°C  7.074W    N/A, N/A, 0         N/A   800Mhz  0%   auto  N/A     81%    0%    
====================================================================================================================
=============================================== End of ROCm SMI Log ================================================

$ which vulkaninfo glxinfo clinfo eglinfo
/usr/bin/vulkaninfo
/usr/bin/glxinfo
/opt/rocm/bin/clinfo
/usr/bin/eglinfo

$ ls /usr/share/vulkan/icd.d/ 2>&1
lvp_icd.json
radeon_icd.json

$ for f in /usr/share/vulkan/icd.d/*.json; do echo "$f"; cat "$f"; done
/usr/share/vulkan/icd.d/lvp_icd.json
{
    "ICD": {
        "api_version": "1.4.354",
        "library_path": "libvulkan_lvp.so"
    },
    "file_format_version": "1.0.1"
}
/usr/share/vulkan/icd.d/radeon_icd.json
{
    "ICD": {
        "api_version": "1.4.354",
        "library_path": "libvulkan_radeon.so"
    },
    "file_format_version": "1.0.1"
}

$ env | grep -iE 'wsl|vulkan|mesa|wgpu|dxg'
```

The final `env | grep` command exited `1` and printed no lines.

Additional device evidence: `/sys/class/drm/renderD128/device/vendor` is
`0x1002`, device is `0x1681`, and its sysfs `dev` value is `226:128`;
`uevent` reports `DRIVER=amdgpu` and `PCI_ID=1002:1681`. `/dev/dri` itself is
absent in this environment. `ldd /usr/lib/libvulkan_radeon.so` resolves its
`libLLVM.so.22.1` dependency. The lavapipe ICD instead has an unresolved
`libLLVM.so.23.1` dependency.

## H1 — Host classification — C5 in the sandbox; host GPU present

**[VERIFIED]** The host-visible system has an AMD Rembrandt Radeon 680M,
`amdgpu`, a Radeon Vulkan ICD, and a DRM render-node entry in sysfs. **[VERIFIED]**
The ordinary sandbox reports `container-other` and has no `/dev/dri` device
node, so it is C5. This classification applies to the sandbox; it is not C1
(no GPU), C2 (no driver), or C3 (no ICD). Approved escalated commands run
outside that sandbox and can access the host adapters.

## H2 — WGPU selection attempts — DONE

An adapter inventory example already exists at
`crates/horizon-client/examples/wgpu_adapter_inventory.rs`; no source changes
were needed. It enumerates `Backends::all()` and prints adapter name, type,
backend, and shader-f64 support.

The first four inventories were run inside the sandbox and each printed only:
`llvmpipe (LLVM 22.1.8, 256 bits) (Cpu, Gl), shader_f64=false`.

| Command | Result |
|---|---|
| `cargo run -p horizon-client-lib --example wgpu_adapter_inventory` | **[VERIFIED]** llvmpipe only |
| `WGPU_BACKEND=vulkan cargo run -p horizon-client-lib --example wgpu_adapter_inventory` | **[VERIFIED]** llvmpipe only |
| `WGPU_BACKEND=gl cargo run -p horizon-client-lib --example wgpu_adapter_inventory` | **[VERIFIED]** llvmpipe only |
| `WGPU_BACKEND=vulkan WGPU_POWER_PREF=high cargo run -p horizon-client-lib --example wgpu_adapter_inventory` | **[VERIFIED]** llvmpipe only |

The inventory was rerun outside the sandbox after host access was approved.
Each of the same four commands (no env, Vulkan, GL, Vulkan plus high power
preference) then enumerated exactly **[VERIFIED]** two hardware adapters.
Outside-sandbox `vulkaninfo --summary` also listed GPU0 as
`AMD Radeon Graphics (RADV REMBRANDT)`, `PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU`,
driver `radv`, Mesa `26.2.1-arch3.1`, Vulkan API `1.4.354`:

| Adapter | Device type/backend | `shader_f64` |
|---|---|---|
| **[VERIFIED]** AMD Radeon Graphics (RADV REMBRANDT) | IntegratedGpu / Vulkan | **[VERIFIED]** true |
| **[VERIFIED]** AMD Radeon Graphics (radeonsi, rembrandt, ACO, DRM 3.64, 7.2.0-1-cachyos-lto) | Other / GL | **[VERIFIED]** false |

**[VERIFIED]** The `WGPU_BACKEND` and `WGPU_POWER_PREF` inventory variants do
not filter `enumerate_adapters(Backends::all())`; all list both adapters.

The environment assignments were command-scoped and did not persist. The
earlier sandbox Vulkan tools could not report an adapter name. The outside-
sandbox WGPU inventory supplied names for `WGPU_ADAPTER_NAME`. I added
adapter-selection environment handling to the five probe entry points using
wgpu's `initialize_adapter_from_env_or_default`. All five probes selected
llvmpipe successfully in a selector check, then the complete suite was run on
both Radeon adapters.

Exact selector-validation commands and results:

The initial five commands below are selector checks on WGPU/Naga `30.0.1`,
adapter `llvmpipe (LLVM 22.1.8, 256 bits)`, backend `Gl`, with default shader
compilation options and `WGPU_ADAPTER_NAME=llvmpipe`.

| Command | Result |
|---|---|
| `WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example wgpu_perlin_probe` | **[VERIFIED]** llvmpipe; Perlin and Smeared Perlin each 0/60,000 |
| `WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example wgpu_lattice_probe` | **[VERIFIED]** llvmpipe; total lattice mismatches 0/60,000 |
| `WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example wgpu_density_interp_probe` | **[VERIFIED]** llvmpipe; interpolation and blend mismatches 0/60,000 |
| `WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example wgpu_density_graph_probe` | **[VERIFIED]** llvmpipe; strict and explicit-FMA graph stages each 0/60,000 |
| `WGPU_ADAPTER_NAME=llvmpipe cargo run -p horizon-client-lib --example wgpu_sqrt_probe` | **[VERIFIED]** llvmpipe; sqrt 232/60,006 |

### H7 — Hardware probe suite — DONE; parity failures captured

All counts below were produced by WGPU/Naga `30.0.1`, wgpu default device
request and default shader compilation options. `WGPU_ADAPTER_NAME` selected
the named adapter. The Vulkan run used the `RADV REMBRANDT` substring; the GL
run used `radeonsi`. Interpolation and graph examples exit nonzero on their
parity assertions; their mismatch reports were captured before exit.

| Stage | RADV REMBRANDT / Vulkan / `shader_f64=true` | radeonsi / GL / `shader_f64=false` |
|---|---:|---:|
| Perlin | **[VERIFIED]** 50,656/60,000 | **[VERIFIED]** 50,296/60,000 |
| Smeared Perlin | **[VERIFIED]** 48,157/60,000 | **[VERIFIED]** 47,709/60,000 |
| Lattice stages | **[VERIFIED]** 0/60,000 | **[VERIFIED]** 0/60,000 |
| Trilinear interpolation | **[VERIFIED]** 27,315/60,000 | **[VERIFIED]** 27,315/60,000 |
| Density blend | **[VERIFIED]** 8,190/60,000 | **[VERIFIED]** 8,190/60,000 |
| Compiled spline | **[VERIFIED]** 671/60,000 | **[VERIFIED]** 671/60,000 |
| Spline plus blend | **[VERIFIED]** 20,651/60,000 | **[VERIFIED]** 20,651/60,000 |
| Explicit-FMA spline | **[VERIFIED]** 1,899/60,000 | **[VERIFIED]** 1,899/60,000 |
| Explicit-FMA spline plus blend | **[VERIFIED]** 1,229/60,000 | **[VERIFIED]** 1,229/60,000 |
| Arbitrary-bit sqrt | **[VERIFIED]** 4,767/60,006 | **[VERIFIED]** 4,767/60,006 |

The original WGPU-generated Perlin table supplied the same CPU reference on
both adapters. The first Perlin mismatch on both was sample **[VERIFIED]** 0:
GPU `0x3e0aa9bd`, CPU `0x3e0aa9b6`. Its input coordinates are
**[INFERRED]** `[16,777,215, -64, -16,777,216]`, from the deterministic sample
construction in the probe. Smeared Perlin had the same first sample and bits.

The first lattice divergence was **[VERIFIED]** none on either adapter. The
first interpolation divergence on both was sample **[VERIFIED]** 0 with
`alpha=[0.0001332617, 0.72219574, 0.0008950914]`, expected
`0x4002a854`, GPU `0x4002a855`. Density blend first diverged at sample
**[VERIFIED]** 4 with `alpha=[0.009571034, 0.026918074, 0.04187827]`,
expected `0xc4d3a764`, GPU `0xc4d3a765`.

For graph stages, both adapters first differed on the ordinary spline at
sample **[VERIFIED]** 84, input `[0.18076034, -0.008255455, 0.20529924]`,
CPU `0x3f11e1ec`, GPU `0x3f11e1ed`. Spline-plus-blend first differed at
sample **[VERIFIED]** 7, input `[3.0, -0.20508145, 0.177807]`, CPU
`0x3e5d483b`, GPU `0x3e5d483a`. Explicit-FMA spline and blend first differed
at sample **[VERIFIED]** 66, input `[-0.16800003, 0.00025177147,
0.037311606]`, CPU spline `0x3edd5a8d`, GPU spline `0x3edd5a8e`; the CPU/GPU
blend bits were `0x3c8550da` / `0x3c8550db`.

The Perlin FMA diagnostic had no fully matching tested CPU formula on Vulkan;
on GL, CPU `fma(lerp)=true`, `fma(smooth inner)=true`, and
`fma(smooth outer)=true` matched all GPU samples (**[VERIFIED]** 0/60,000
against GPU, while differing from the Rust Perlin reference on 50,296 samples).
The explicit-FMA graph variant that matched llvmpipe did not match either AMD
backend. This is a backend-specific arithmetic contract failure, not a
sampling threshold or tolerance issue.

The sqrt rows are separate from Perlin. Each AMD adapter had **[VERIFIED]**
4,557 finite mismatches: 127 subnormal inputs flushed to zero and 4,430
positive normal inputs differed; 105 negative inputs produced finite GPU
outputs. Maximum reported finite error was **[VERIFIED]** 536,804,736 ULP and
`1.0995116e12` absolute.

Sandbox-only Vulkan/GL evidence from H0 (the host-side Vulkan retry succeeded):

- `vulkaninfo --summary` failed with `Failed to detect any valid GPUs in the
  current config`; it also reported that lavapipe could not load
  `libLLVM.so.23.1`.
- `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/radeon_icd.json vulkaninfo --summary`
  failed with `Failed to detect any valid GPUs in the current config`.
- Forcing the lavapipe ICD with `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`
  failed because `libLLVM.so.23.1` is absent.
- `glxinfo -B` failed because display `:0` is unavailable. `eglinfo -B`
  initialized only its surfaceless platform and reported llvmpipe.

WGPU and Naga resolved to version `30.0.1` in this workspace. The hardware
probe suite ran on both host adapters; results and first divergences follow.

## H5 — Container/VM path — sandbox blocked; host retry succeeded

**[VERIFIED]** The ordinary sandbox has `/sys/class/drm/renderD128` but no
`/dev/dri` device node, and reports `container-other`. This prevented GPU
access from sandboxed commands. An approved command running outside the
sandbox enumerated both AMD adapters and ran the complete probe suite on each.
No device mapping, driver, ICD, or persistent permission change was made.

## Reproduction commands for a GPU-enabled host

From the Horizon checkout, run the inventory and the complete existing probe
suite with Vulkan/high-performance selection:

```sh
set -u
cargo run -p horizon-client-lib --example wgpu_adapter_inventory
failed=
for probe in \
  wgpu_perlin_probe \
  wgpu_lattice_probe \
  wgpu_density_interp_probe \
  wgpu_density_graph_probe \
  wgpu_sqrt_probe
do
  if ! WGPU_BACKEND=vulkan WGPU_POWER_PREF=high \
    cargo run -p horizon-client-lib --example "$probe"
  then
    failed="$failed $probe"
  fi
done
if [ -n "$failed" ]; then
  echo "probes failed:$failed" >&2
  exit 1
fi
```

To repeat for every physical adapter, set `WGPU_ADAPTER_NAME` to each
case-insensitive name substring printed by the inventory and run the loop
again. Confirm each probe's printed adapter before treating the result as
hardware parity.

## Change and rollback record

- **[VERIFIED]** No packages, drivers, ICDs, container settings, source files,
  or persistent environment variables were changed.
- **[VERIFIED]** `WGPU_BACKEND`, `WGPU_POWER_PREF`, `WGPU_ADAPTER_NAME`, and
  `VK_ICD_FILENAMES` were
  supplied only as prefixes to individual commands. They require no rollback;
  there are no exported settings to unset.
- **[VERIFIED]** The only source changes are adapter-selection environment
  handling in the five probe entry points; no dependencies were added. They
  can be reverted by removing the calls to
  `wgpu::util::initialize_adapter_from_env_or_default` and restoring each
  entry point's prior default `request_adapter` call. This rollback was not
  applied.
