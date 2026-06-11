# aivpn-linux-kernel — Technical Design Document

## 1. Purpose and Scope

This document specifies the design for an optional Linux kernel module that accelerates the AIVPN data path. The module is **not required**; both `aivpn-server` and `aivpn-client` detect its presence at startup and fall back transparently to the existing user-space Tokio/TUN implementation when it is absent or fails to load.

The acceleration target is the per-packet hot path:

```
[NIC RX] → UDP recv → tag lookup → ChaCha20-Poly1305 decrypt → TUN write → [app]
[app]    → TUN read → tag gen    → ChaCha20-Poly1305 encrypt → UDP send → [NIC TX]
```

In the current user-space implementation this path crosses the kernel/user boundary **four times per packet** (UDP recv, TUN write, TUN read, UDP send) and performs one `copy_to_user` / `copy_from_user` on each crossing. The kernel module eliminates all four crossings on the forwarding path and reduces the crypto path to a single in-kernel AEAD call.

---

## 2. Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                    Linux kernel                           │
│                                                          │
│  ┌─────────────┐   ┌──────────────────────────────────┐  │
│  │  UDP socket  │   │       aivpn.ko                   │  │
│  │  (bound by   │──▶│  ┌─────────────────────────────┐ │  │
│  │  user-space  │   │  │  tag → session hashtable    │ │  │
│  │  or module)  │   │  │  (DEFINE_HASHTABLE, 8-byte) │ │  │
│  └─────────────┘   │  └────────────┬────────────────┘ │  │
│                    │               │ session found      │  │
│                    │  ┌────────────▼────────────────┐  │  │
│                    │  │  crypto_aead (chacha20poly1305│  │  │
│                    │  │  via kernel crypto API)       │  │  │
│                    │  └────────────┬────────────────┘  │  │
│                    │               │ plaintext          │  │
│                    │  ┌────────────▼────────────────┐  │  │
│                    │  │  tun_net_xmit / netif_rx    │  │  │
│                    │  │  (skb hand-off to TUN dev)  │  │  │
│                    │  └─────────────────────────────┘  │  │
│                    └──────────────────────────────────┘  │
│                                                          │
│  ┌───────────────────────────────────────────────────┐   │
│  │  /dev/aivpn  (character device, misc_register)   │   │
│  │  ioctl interface for session CRUD from user space │   │
│  └───────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
         ▲                              ▲
         │ startup ioctl                │ fallback if ENODEV
         │                              │
┌────────┴──────────────────────────────┴────────────┐
│          aivpn-server / aivpn-client               │
│  KernelAccel::try_new() → Some(accel) or None      │
└────────────────────────────────────────────────────┘
```

User-space interaction is limited to the **control plane** (session install/remove via ioctl on `/dev/aivpn`). The **data plane** runs entirely in-kernel: the module registers a UDP `proto_ops`-level hook (or an `sk_buff` filter on the bound socket) and routes decrypted packets directly into the TUN net device via `netif_rx_ni`.

---

## 3. Why a Character Device + ioctl, Not Netlink

| Criterion | `/dev/aivpn` + ioctl | Netlink |
|---|---|---|
| Latency of control-plane calls | `O(1)`, single syscall | Requires socket, `sendmsg`, `recvmsg` round-trip |
| API versioning | `_IOC` magic encodes version | NLMSG type can do the same but more boilerplate |
| Privilege separation | `open(O_RDWR)` + DAC/LSM on the device file | Requires `CAP_NET_ADMIN` netlink family |
| Fit for existing code | Single `fd = open("/dev/aivpn")` + `ioctl` calls mirror pattern of `/dev/net/tun` which the codebase already uses in `tunnel.rs` | New async netlink socket machinery needed |
| Complexity | ~200 lines for the fops + ioctl dispatch | ~500 lines for policy, parsing, attribute TLVs |
| Inspection / debugging | `strace` sees each ioctl immediately | Requires `ss -N` or wireshark netlink dissector |

The codebase already opens `/dev/net/tun` in `tunnel.rs`. Adding `/dev/aivpn` follows the same idiom; engineers familiar with the TUN code will immediately understand the new device.

Netlink would be appropriate if the module needed to multicast events (e.g., session expiry notifications) to multiple listeners. For this use case — unidirectional session CRUD from a single process — ioctl is the right tool.

---

## 4. Kernel API Surface

### 4.1 Device Node

```
/dev/aivpn   (major: misc, minor: dynamic via misc_register)
permissions: 0600, owned by root
```

### 4.2 ioctl Commands

All structures are packed; `__u8`, `__u32`, `__u64` types from `<linux/types.h>`.

```c
#define AIVPN_MAGIC  0xAE

/* Install a new session into the kernel session table */
#define AIVPN_IOC_SESSION_ADD    _IOW(AIVPN_MAGIC, 1, struct aivpn_session_add)

/* Remove a session by its 16-byte session_id */
#define AIVPN_IOC_SESSION_DEL    _IOW(AIVPN_MAGIC, 2, struct aivpn_session_del)

/* Query liveness / packet counters for a session */
#define AIVPN_IOC_SESSION_STAT   _IOWR(AIVPN_MAGIC, 3, struct aivpn_session_stat)

/* Register the TUN ifindex that decrypted packets should be injected into */
#define AIVPN_IOC_SET_TUN        _IOW(AIVPN_MAGIC, 4, struct aivpn_set_tun)

/* Register the UDP socket fd whose RX the module should intercept */
#define AIVPN_IOC_SET_UDP_SOCK   _IOW(AIVPN_MAGIC, 5, struct aivpn_set_udp_sock)

/* Flush all sessions (called on clean shutdown) */
#define AIVPN_IOC_FLUSH          _IO(AIVPN_MAGIC, 6)

/* Get module API version */
#define AIVPN_IOC_GET_VERSION    _IOR(AIVPN_MAGIC, 7, __u32)
```

Key session-add payload fields (see `include/uapi/aivpn.h`):

```c
struct aivpn_session_add {
    __u8  session_id[16];
    __u8  session_key[32];   /* ChaCha20-Poly1305 key */
    __u8  tag_secret[32];    /* BLAKE3 tag derivation secret */
    __u8  prng_seed[32];     /* PRNG seed for nonce generation */
    __u64 counter_base;      /* current send_counter */
    __u32 client_ip;         /* VPN IPv4 address (network byte order) */
    __u8  client_addr[28];   /* struct sockaddr_storage for UDP peer */
    __u64 window_ms;         /* tag validity window (DEFAULT_WINDOW_MS = 10000) */
};
```

### 4.3 Data-Plane Hook

After `AIVPN_IOC_SET_UDP_SOCK` the module installs a `sk_data_ready` callback replacement on the socket. Incoming `sk_buff`s bypass `recvmsg`; the module consumes them directly:

1. Read 8-byte resonance tag from buffer head.
2. Hash-table lookup: `hash_32(tag_lo ^ tag_hi, AIVPN_HASH_BITS)` → linked-list walk.
3. Validate tag against time window (replicate `validate_tag` logic from `session.rs`).
4. Call `crypto_aead_decrypt` (synchronous, in-module).
5. Prepend 4-byte TUN PI header (`ETH_P_IP`).
6. `netif_rx_ni(skb)` into the registered TUN net device.

The reverse path (TUN → UDP encrypt → send) is handled by a `ndo_start_xmit` override on the TUN device's `net_device_ops`.

---

## 5. Auto-Detection and Fallback

### 5.1 KernelAccel Trait

A new trait in `aivpn-common` (or a `kernel_accel` feature-gated module in each binary) defines the interface:

```rust
/// Attempt to open /dev/aivpn and verify the module API version.
/// Returns None on ENODEV, EPERM, or version mismatch.
pub trait KernelAccel: Sized {
    fn try_new() -> Option<Self>;
    fn install_session(&self, session: &KernelSession) -> std::io::Result<()>;
    fn remove_session(&self, session_id: &[u8; 16]) -> std::io::Result<()>;
    fn set_tun_ifindex(&self, ifindex: u32) -> std::io::Result<()>;
    fn set_udp_socket(&self, fd: RawFd) -> std::io::Result<()>;
    fn flush(&self) -> std::io::Result<()>;
}

pub struct KernelSession {
    pub session_id:   [u8; 16],
    pub session_key:  [u8; 32],
    pub tag_secret:   [u8; 32],
    pub prng_seed:    [u8; 32],
    pub counter_base: u64,
    pub client_ip:    Ipv4Addr,
    pub client_addr:  SocketAddr,
    pub window_ms:    u64,
}
```

`try_new()` implementation:

```rust
fn try_new() -> Option<Self> {
    let fd = match std::fs::OpenOptions::new()
        .read(true).write(true).open("/dev/aivpn") {
        Ok(f) => f,
        Err(e) if e.raw_os_error() == Some(libc::ENODEV)
               || e.raw_os_error() == Some(libc::ENOENT) => return None,
        Err(e) => {
            tracing::warn!("kernel accel unavailable: {e}");
            return None;
        }
    };
    let version = unsafe { ioctl_get_version(fd.as_raw_fd()) };
    if version != AIVPN_MODULE_API_VERSION { return None; }
    Some(Self { fd })
}
```

### 5.2 Integration Points in aivpn-server

**`gateway.rs`** — the `GatewayEngine` struct gains an optional field:

```rust
struct GatewayEngine {
    // ... existing fields ...
    kernel_accel: Option<Box<dyn KernelAccelTrait>>,
}
```

In `GatewayEngine::new()`, before binding the UDP socket:

```rust
let kernel_accel = KernelAccelImpl::try_new();
if kernel_accel.is_some() {
    info!("kernel acceleration enabled via /dev/aivpn");
} else {
    info!("kernel acceleration unavailable, using user-space path");
}
```

In `SessionManager::create_session()` (called from `gateway.rs` on handshake completion), after inserting into the `DashMap`:

```rust
if let Some(ref accel) = self.kernel_accel {
    let ks = KernelSession::from_session(&session);
    if let Err(e) = accel.install_session(&ks) {
        warn!("failed to install session into kernel: {e}");
        // non-fatal: packet will still be handled in user space
    }
}
```

The main Tokio event loop in `gateway.rs` checks `kernel_accel.is_some()` to decide whether to run the `udp_socket.recv_from` loop. When the module is active the UDP socket read loop **exits** — the kernel handles all RX. The gateway still runs the TUN-read loop for outbound traffic unless `AIVPN_IOC_SET_TUN` was successfully called (in which case the kernel handles that direction too).

Session cleanup mirrors session creation: `accel.remove_session(&session.session_id)` is called in `SessionManager::remove_session()`.

### 5.3 Integration Points in aivpn-client

**`client.rs`** — the `AivpnClient` state machine checks for kernel accel after the TUN device is created in `tunnel.rs` and before entering the `Connected` state:

```rust
// In AivpnClient::on_connected():
let accel = KernelAccelImpl::try_new();
if let Some(ref accel) = accel {
    accel.set_tun_ifindex(self.tun.ifindex())?;
    accel.set_udp_socket(self.udp_sock.as_raw_fd())?;
    accel.install_session(&KernelSession::from(&self.session_keys, ...))?;
    info!("kernel acceleration active");
}
self.kernel_accel = accel;
```

On `KeyRotate` / ratchet completion (`complete_ratchet()` equivalent in client), the old session is removed and the new one installed atomically.

---

## 6. Crypto Path

### 6.1 Algorithm Mapping

The user-space codebase uses the `chacha20poly1305` crate with a 32-byte key and 12-byte nonce (see `crypto.rs`: `CHACHA20_KEY_SIZE=32`, `NONCE_SIZE=12`, `POLY1305_TAG_SIZE=16`). This maps directly to the kernel algorithm name:

```
"rfc7539(chacha20,poly1305)"
```

or on kernels that expose it as a single AEAD:

```
"chacha20poly1305"
```

The kernel `crypto_aead` API (synchronous variant `crypto_alloc_aead`) is used. Each session holds one `crypto_aead *` handle allocated at session-install time. The key is set once via `crypto_aead_setkey`. The authsize is 16 bytes (Poly1305 tag), set via `crypto_aead_setauthsize`.

### 6.2 AF_ALG vs In-Module

Two implementation strategies exist:

| Strategy | Description | Pros | Cons |
|---|---|---|---|
| **AF_ALG** | User space sends data through `AF_ALG` socket; kernel crypto runs on the other side | No kernel module needed for crypto itself | Still crosses user/kernel boundary per packet — defeats the purpose |
| **In-module `crypto_aead`** | Module calls `crypto_aead_decrypt`/`encrypt` directly inside `sk_data_ready` | Zero crossings; skb can be passed through without copy if IV/AAD fit in headroom | Requires `crypto_alloc_aead` at module init; AEAD transform must be available |

**Decision: in-module `crypto_aead`**. The AF_ALG approach is appropriate for user-space offload but not for a bypass path. The kernel's `chacha20poly1305` software implementation is available on all architectures (it is the WireGuard cipher too); hardware offload (e.g., ARMv8 crypto extensions) is used automatically by the kernel's algorithm selection layer.

### 6.3 Nonce Construction

The nonce (12 bytes) is constructed the same way as in user-space `encrypt_payload` / `decrypt_payload`:

```
nonce[0..8]  = send_counter (little-endian u64)
nonce[8..12] = prng_seed[0..4]
```

The kernel session struct caches the current counter value; it is incremented atomically using `atomic64_inc_return`.

### 6.4 AAD (Additional Authenticated Data)

The 8-byte resonance tag itself serves as the AAD, matching the user-space convention. This ensures an attacker cannot swap tags between sessions.

---

## 7. Session Table

### 7.1 Current User-Space Approach

`session.rs` uses `DashMap<[u8; TAG_SIZE], SessionId>` for O(1) tag-to-session lookup. `DashMap` is a sharded concurrent hash map from the `dashmap` crate.

### 7.2 Kernel Hashtable Design

The kernel `DEFINE_HASHTABLE` macro creates a statically-sized bucket array with chaining. For the 500-session maximum:

```c
/* 512 buckets → average chain length 1 at MAX_SESSIONS=500 */
#define AIVPN_HASH_BITS 9
DEFINE_HASHTABLE(aivpn_session_table, AIVPN_HASH_BITS);

struct aivpn_kern_session {
    __u8   tag[8];           /* current resonance tag (key) */
    __u8   session_id[16];
    __u8   session_key[32];
    __u8   tag_secret[32];
    __u8   prng_seed[32];
    __u64  counter;          /* atomic, use atomic64_t */
    __u64  window_ms;
    __u32  client_ip;
    struct sockaddr_storage client_addr;
    struct crypto_aead *tfm; /* per-session AEAD transform */
    struct hlist_node  hnode;
    spinlock_t         lock;
    /* stats */
    __u64  rx_packets;
    __u64  tx_packets;
    __u64  rx_bytes;
    __u64  tx_bytes;
};
```

Hash function: `jhash(tag, 8, 0)` — consistent with kernel network code.

The global table is protected by a `rwlock_t`; individual session structs have their own `spinlock_t` for counter / stats updates without blocking the global reader lock.

### 7.3 Tag Window Validation

The tag window logic from `session.rs` (`TAG_WINDOW_SIZE=256`, `DEFAULT_WINDOW_MS=10_000`) is replicated in the kernel:

```c
static bool aivpn_tag_valid(struct aivpn_kern_session *s,
                             const __u8 *tag, __u64 now_ms)
{
    __u64 tag_ts;
    /* Extract embedded timestamp from tag (first 8 bytes carry truncated ms) */
    memcpy(&tag_ts, tag, sizeof(tag_ts));
    tag_ts = le64_to_cpu(tag_ts);
    return (now_ms - tag_ts) <= s->window_ms;
}
```

This is a simplified view; the actual bitmap anti-replay window from `session.rs` (`received_bitmap: u256`) is replicated as a 256-bit bitmap using four `unsigned long` words.

---

## 8. DKMS Integration

### 8.1 Minimum Kernel Requirements

| Requirement | Minimum version | Notes |
|---|---|---|
| `CONFIG_RUST=y` | 6.1 | Rust-for-Linux merged in 6.1 |
| `crypto_aead` / `chacha20poly1305` | 4.10 | Available in all targets |
| `misc_register` | 2.6 | Always available |
| `DEFINE_HASHTABLE` | 3.7 | Available everywhere |
| Rust `kernel` crate stable enough for net | ~6.8–6.9 | See §9 — C is used for data plane |

**Minimum supported kernel: 6.1** for Rust abstractions in the control-plane glue. The data-plane C code works on kernels as old as 4.10 if the Rust wrapper is not used.

### 8.2 `dkms.conf`

```
PACKAGE_NAME="aivpn"
PACKAGE_VERSION="@VERSION@"
CLEAN="make clean"
MAKE[0]="make -C /lib/modules/${kernelver}/build M=${dkms_tree}/${PACKAGE_NAME}/${PACKAGE_VERSION}/build"
BUILT_MODULE_NAME[0]="aivpn"
BUILT_MODULE_LOCATION[0]="."
DEST_MODULE_LOCATION[0]="/kernel/net/aivpn/"
STRIP[0]="no"
AUTOINSTALL="yes"
```

### 8.3 `Makefile` Stub

```makefile
KVER    ?= $(shell uname -r)
KDIR    ?= /lib/modules/$(KVER)/build
PWD     := $(shell pwd)

obj-m   := aivpn.o
aivpn-y := src/main.o src/session_table.o src/crypto_ops.o \
            src/tun_inject.o src/dev.o src/udp_hook.o

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

install:
	$(MAKE) -C $(KDIR) M=$(PWD) modules_install
	depmod -a
```

---

## 9. Rust-for-Linux Abstractions

The module uses a **hybrid approach**: the critical data-plane hot path is written in C for stability, while the control-plane `file_operations` (ioctl dispatch, module init/exit) use the Rust-for-Linux `kernel` crate. This boundary is motivated by:

- Rust kernel abstractions for `net::*` (socket hooks, `sk_buff`) are not yet stable as of kernel 6.9 — they are behind `#[cfg(CONFIG_RUST)]` feature flags and the API changes between kernel versions.
- The C data-plane code is ~400 lines and straightforward to audit.
- The Rust control-plane code benefits from memory-safety guarantees in the ioctl parsing / session-struct management where pointer arithmetic is dense.

### 9.1 Rust Abstractions Used

```rust
use kernel::prelude::*;
use kernel::sync::Mutex;          // wraps raw spinlock/mutex for session table access
use kernel::io_buffer::{IoBufferReader, IoBufferWriter}; // safe ioctl copy_from/to_user
use kernel::file_operations::{FileOperations, IoctlCommand}; // register fops
use kernel::miscdev::Registration; // misc_register equivalent
```

`kernel::net` abstractions (`UdpSocket`, `SkBuff`) are conditionally compiled and guarded by a version check. If unavailable, the C shim layer is used for the UDP hook.

### 9.2 Instability Warning

The `kernel` crate API is **unstable and changes with each kernel release**. The `Makefile` binds to the exact kernel version; DKMS rebuilds on kernel upgrade. Any kernel crate API breakage surfaces as a build error at DKMS install time, which is preferable to a runtime panic. The design avoids all `unsafe` in Rust except where the `kernel` crate itself requires it (raw pointer FFI to C helpers).

---

## 10. Full Directory Tree

```
aivpn-linux-kernel/
├── DESIGN.md                  ← this document
├── README.md                  ← user-facing install guide
├── dkms.conf                  ← DKMS package descriptor (version substituted by build script)
├── Makefile                   ← kernel module build entry point
├── Kbuild                     ← alternative obj-m list (used by kbuild when M= is set)
├── include/
│   └── uapi/
│       └── aivpn.h            ← shared userspace/kernel header (ioctl defs, structs)
├── src/
│   ├── main.rs                ← Rust: module_init / module_exit, misc device registration
│   ├── dev.rs                 ← Rust: FileOperations impl, ioctl dispatch, copy_from/to_user
│   ├── session_table.c        ← C: DEFINE_HASHTABLE, session insert/lookup/remove, tag window
│   ├── session_table.h        ← C: struct aivpn_kern_session, function prototypes
│   ├── crypto_ops.c           ← C: crypto_alloc_aead, per-session encrypt/decrypt wrappers
│   ├── crypto_ops.h           ← C: prototypes for crypto_ops.c
│   ├── udp_hook.c             ← C: sk_data_ready hook, sk_buff RX intercept
│   ├── udp_hook.h             ← C header
│   ├── tun_inject.c           ← C: netif_rx_ni into TUN device, ndo_start_xmit override
│   ├── tun_inject.h           ← C header
│   └── helpers.h              ← C: shared macros, printk wrappers, version compat shims
├── scripts/
│   ├── install-dkms.sh        ← install DKMS source tree, run dkms add/build/install
│   └── uninstall-dkms.sh      ← dkms remove + rmmod
└── tests/
    ├── user_ioctl_test.c      ← userspace smoke test: open /dev/aivpn, ioctl add/stat/del
    └── Makefile               ← build test binary against include/uapi/aivpn.h
```

### Source File Responsibilities

| File | Language | Responsibility |
|---|---|---|
| `src/main.rs` | Rust | `module!` macro, `module_init`/`module_exit`, `misc_register`, global state init |
| `src/dev.rs` | Rust | `FileOperations`: `open`, `release`, `ioctl`; safe parsing of ioctl payloads via `IoBufferReader`; dispatches to C helpers |
| `src/session_table.c` | C | `DEFINE_HASHTABLE` (512 buckets); `aivpn_session_insert`, `aivpn_session_lookup`, `aivpn_session_remove`, `aivpn_session_flush`; tag-window bitmap validation; `rwlock_t` global + per-session `spinlock_t` |
| `src/session_table.h` | C | `struct aivpn_kern_session`; function prototypes |
| `src/crypto_ops.c` | C | `aivpn_crypto_init_session` (calls `crypto_alloc_aead("rfc7539(chacha20,poly1305)")`); `aivpn_crypto_decrypt`; `aivpn_crypto_encrypt`; nonce construction matching user-space `encrypt_payload` |
| `src/udp_hook.c` | C | Saves original `sk->sk_data_ready`; installs `aivpn_sk_data_ready`; on RX: extracts tag, calls `aivpn_session_lookup`, calls `aivpn_crypto_decrypt`, passes skb to `aivpn_tun_inject` |
| `src/tun_inject.c` | C | Holds `net_device *` reference for the TUN device; `aivpn_tun_inject`: prepends PI header, calls `netif_rx_ni`; `aivpn_tun_xmit`: TUN `ndo_start_xmit` override for TX path (encrypt → UDP send via `kernel_sendmsg`) |
| `src/helpers.h` | C | `aivpn_dbg` / `aivpn_err` wrappers around `pr_debug` / `pr_err`; `ktime_get_ms()` shim; version compat `#if LINUX_VERSION_CODE` guards |
| `include/uapi/aivpn.h` | C | All `AIVPN_IOC_*` ioctl defines, `struct aivpn_session_add`, `struct aivpn_session_del`, `struct aivpn_session_stat`, `struct aivpn_set_tun`, `struct aivpn_set_udp_sock`; included by both kernel and user space |
| `tests/user_ioctl_test.c` | C | Opens `/dev/aivpn`, installs a synthetic session with known key, checks `AIVPN_IOC_SESSION_STAT`, removes session; exits 0 on success |

---

## 11. Performance Expectations

### 11.1 Current User-Space Path (per packet, inbound)

| Step | Mechanism | Cost |
|---|---|---|
| UDP `recv_from` | `recvmsg` syscall + `copy_to_user` | ~200–400 ns + copy |
| Tag lookup | `DashMap` shard lock + hash | ~50–100 ns |
| `decrypt_payload` | `chacha20poly1305` crate (software) | ~100–300 ns per 1500-byte packet |
| TUN `write` | `write` syscall + `copy_from_user` | ~200–400 ns + copy |
| **Total crossings** | **4 (2 syscalls × 2 copies)** | |

At 1 Gbps line rate with 1500-byte packets: ~83,000 pps. Each `copy_to_user` + `copy_from_user` pair costs ~800 ns on a modern 3 GHz core → **~66 µs overhead per 1000 packets**, or ~5.5 ms/s at 83 kpps — roughly 0.5% CPU on one core at 1 Gbps. At 10 Gbps (830 kpps) this becomes 5% of one core just in copy overhead, before crypto.

### 11.2 Kernel Module Path (per packet, inbound)

| Step | Mechanism | Cost |
|---|---|---|
| `sk_data_ready` hook | Called in softirq context; no syscall | ~10 ns |
| Tag lookup | `jhash` + `hlist` walk under `read_lock` | ~30–60 ns |
| `crypto_aead_decrypt` | In-kernel ChaCha20-Poly1305 (same SW impl, possibly SIMD via kernel glue) | ~80–250 ns per 1500-byte packet |
| `netif_rx_ni` into TUN | Direct `skb` hand-off; no copy if headroom is pre-allocated | ~20–50 ns |
| **Total crossings** | **0 (stays in kernel)** | |

### 11.3 Zero-Copy Condition

Zero-copy is achieved when:

1. The UDP socket uses `SO_ZEROCOPY` or the driver supports `XDP` and the skb headroom >= 4 bytes (TUN PI header).
2. The crypto decrypt is done in-place on the existing skb data area (requires the AEAD implementation to support in-place operation — `rfc7539` does).
3. The decrypted skb is passed to `netif_rx_ni` directly without `skb_copy`.

Under these conditions, the inbound path from NIC DMA to TUN device involves **zero extra copies** — the packet buffer is owned by the kernel from arrival to delivery.

### 11.4 Expected Throughput Gain

Benchmarks against analogous kernel-bypass VPN implementations (WireGuard vs. OpenVPN user-space) show 2–5× throughput improvement at high packet rates and 30–60% latency reduction. For AIVPN the gains will be similar:

| Metric | User-space | Kernel module | Expected gain |
|---|---|---|---|
| Throughput (1500B) | ~2 Gbps (1 core) | ~5–8 Gbps (1 core) | 2.5–4× |
| Latency (p50) | ~600–900 µs | ~100–200 µs | 3–5× |
| CPU at 1 Gbps | ~15–25% (1 core) | ~5–8% (1 core) | 3× |

These are projections based on WireGuard performance data and the overhead analysis above. Actual numbers depend on NIC driver, CPU, and whether SIMD ChaCha20 is available in the kernel's crypto layer.

### 11.5 What the Module Does NOT Accelerate

- **Neural Resonance Module** (`neural.rs`): MLP forward-pass remains in user space. The module sends periodic `AIVPN_IOC_SESSION_STAT` ioctls to read packet counts and feeds them to the neural module. If rotation is triggered, user space calls `AIVPN_IOC_SESSION_DEL` + `AIVPN_IOC_SESSION_ADD` with the new session key.
- **Mask mimicry / traffic shaping** (`mimicry.rs`): FSM-driven packet shaping requires timing control that is not appropriate in softirq context. Shaped packets are still handed off to user space via a separate slow path.
- **Key exchange / handshake**: The X25519 + HKDF handshake runs in user space as before. The kernel session is installed only after the handshake completes.
- **Recording** (`recording.rs`): Traffic recording for mask generation remains in user space.

---

## 12. Security Considerations

- **Key material in kernel memory**: Session keys are stored in `struct aivpn_kern_session` which is `kzalloc`-allocated. On `AIVPN_IOC_SESSION_DEL` the struct is `memzero_explicit`-zeroed before `kfree` to prevent key material from lingering in slab caches.
- **ioctl authentication**: `/dev/aivpn` is mode `0600` root-only. A future enhancement could use a `securityfs` policy or LSM hook to restrict access to the specific `aivpn-server` binary by inode.
- **Anti-replay**: The 256-bit bitmap from `session.rs` (`received_bitmap`) is replicated in `struct aivpn_kern_session` as four `unsigned long` words, updated atomically with `test_and_set_bit`.
- **Tag collision**: The existing `subtle::ConstantTimeEq` constant-time comparison in `session.rs` is replicated using `crypto_memneq` in the kernel.
