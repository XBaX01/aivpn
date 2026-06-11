/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
/*
 * aivpn.h — shared userspace/kernel UAPI header
 *
 * Included by both the kernel module (src/*.c) and userspace tools
 * (tests/user_ioctl_test.c, aivpn-server dev.rs FFI layer).
 *
 * All structs are packed to avoid ABI surprises across compilers.
 */

#ifndef _UAPI_AIVPN_H
#define _UAPI_AIVPN_H

#ifdef __KERNEL__
#include <linux/types.h>
#include <linux/ioctl.h>
#else
#include <stdint.h>
#include <sys/ioctl.h>
typedef uint8_t  __u8;
typedef uint16_t __u16;
typedef uint32_t __u32;
typedef uint64_t __u64;
#endif

/* Module API version returned by AIVPN_IOC_GET_VERSION.
 * Increment when any ioctl struct or semantic changes. */
#define AIVPN_MODULE_API_VERSION  1U

/* ioctl magic byte */
#define AIVPN_MAGIC  0xAE

/* ------------------------------------------------------------------ *
 *  Structures                                                          *
 * ------------------------------------------------------------------ */

/**
 * struct aivpn_session_add - payload for AIVPN_IOC_SESSION_ADD
 *
 * @session_id:   16-byte opaque session identifier (matches Rust [u8;16])
 * @session_key:  32-byte ChaCha20-Poly1305 symmetric key
 * @tag_secret:   32-byte BLAKE3 secret used to derive resonance tags
 * @prng_seed:    32-byte PRNG seed; bytes [0..4] used as nonce suffix
 * @counter_base: initial send counter value (little-endian u64)
 * @client_ip:    VPN IPv4 address assigned to this client (network byte order)
 * @client_addr:  28-byte sockaddr_storage holding UDP peer address
 * @window_ms:    tag validity window in milliseconds (default 10000)
 */
struct aivpn_session_add {
	__u8  session_id[16];
	__u8  session_key[32];
	__u8  tag_secret[32];
	__u8  prng_seed[32];
	__u64 counter_base;
	__u32 client_ip;
	__u8  client_addr[28];
	__u64 window_ms;
} __attribute__((packed));

/**
 * struct aivpn_session_del - payload for AIVPN_IOC_SESSION_DEL
 *
 * @session_id: 16-byte session identifier to remove
 */
struct aivpn_session_del {
	__u8 session_id[16];
} __attribute__((packed));

/**
 * struct aivpn_session_stat - payload for AIVPN_IOC_SESSION_STAT
 *
 * Fill session_id before the ioctl; the kernel fills the remaining fields.
 *
 * @session_id: 16-byte session identifier to query
 * @active:     1 if the session exists in the kernel table, 0 otherwise
 * @rx_packets: inbound packet count
 * @tx_packets: outbound packet count
 * @rx_bytes:   inbound byte count
 * @tx_bytes:   outbound byte count
 */
struct aivpn_session_stat {
	__u8  session_id[16];
	__u32 active;
	__u64 rx_packets;
	__u64 tx_packets;
	__u64 rx_bytes;
	__u64 tx_bytes;
} __attribute__((packed));

/**
 * struct aivpn_set_tun - payload for AIVPN_IOC_SET_TUN
 *
 * @ifindex: net_device ifindex of the TUN interface that decrypted
 *           packets should be injected into via netif_rx().
 */
struct aivpn_set_tun {
	__u32 ifindex;
} __attribute__((packed));

/**
 * struct aivpn_set_udp_sock - payload for AIVPN_IOC_SET_UDP_SOCK
 *
 * @fd: file descriptor of the UDP socket whose sk_data_ready the
 *      module should intercept.
 */
struct aivpn_set_udp_sock {
	__u32 fd;
} __attribute__((packed));

/* ------------------------------------------------------------------ *
 *  ioctl commands                                                      *
 * ------------------------------------------------------------------ */

/** Install a new session into the kernel session table */
#define AIVPN_IOC_SESSION_ADD   _IOW(AIVPN_MAGIC, 1, struct aivpn_session_add)

/** Remove a session by its 16-byte session_id */
#define AIVPN_IOC_SESSION_DEL   _IOW(AIVPN_MAGIC, 2, struct aivpn_session_del)

/** Query liveness / packet counters for a session */
#define AIVPN_IOC_SESSION_STAT  _IOWR(AIVPN_MAGIC, 3, struct aivpn_session_stat)

/** Register the TUN ifindex that decrypted packets should be injected into */
#define AIVPN_IOC_SET_TUN       _IOW(AIVPN_MAGIC, 4, struct aivpn_set_tun)

/** Register the UDP socket fd whose RX the module should intercept */
#define AIVPN_IOC_SET_UDP_SOCK  _IOW(AIVPN_MAGIC, 5, struct aivpn_set_udp_sock)

/** Flush all sessions (called on clean shutdown) */
#define AIVPN_IOC_FLUSH         _IO(AIVPN_MAGIC, 6)

/** Get module API version — returns AIVPN_MODULE_API_VERSION */
#define AIVPN_IOC_GET_VERSION   _IOR(AIVPN_MAGIC, 7, __u32)

#endif /* _UAPI_AIVPN_H */
