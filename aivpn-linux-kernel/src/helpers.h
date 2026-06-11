/* SPDX-License-Identifier: GPL-2.0 */
/*
 * helpers.h — shared macros, printk wrappers, and version-compat shims
 *
 * Included by all C source files in the aivpn kernel module.
 * Must not define any global variables or functions (header-only).
 */

#ifndef AIVPN_HELPERS_H
#define AIVPN_HELPERS_H

#include <linux/version.h>
#include <linux/ktime.h>
#include <linux/printk.h>

/* ------------------------------------------------------------------ *
 *  Module-wide constants                                               *
 * ------------------------------------------------------------------ */

/** Log2 of the session hash table bucket count (2^9 = 512 buckets).
 *  512 buckets gives average chain length ~1 at MAX_SESSIONS=500. */
#define AIVPN_HASH_BITS  9

/** Hard cap on concurrent kernel-accelerated sessions. */
#define MAX_SESSIONS     500

/** Size of the resonance tag in bytes (must match TAG_SIZE in crypto.rs). */
#define AIVPN_TAG_SIZE   8

/* ------------------------------------------------------------------ *
 *  Logging helpers                                                     *
 * ------------------------------------------------------------------ */

#define aivpn_dbg(fmt, ...)  pr_debug("aivpn: " fmt, ##__VA_ARGS__)
#define aivpn_info(fmt, ...) pr_info("aivpn: " fmt, ##__VA_ARGS__)
#define aivpn_warn(fmt, ...) pr_warn("aivpn: " fmt, ##__VA_ARGS__)
#define aivpn_err(fmt, ...)  pr_err("aivpn: " fmt, ##__VA_ARGS__)

/* ------------------------------------------------------------------ *
 *  ktime_get_ms() — milliseconds since boot (monotonic)               *
 * ------------------------------------------------------------------ */

/**
 * aivpn_ktime_ms - return current monotonic time in milliseconds.
 *
 * ktime_get_ms() was added in 5.3; provide a shim for older kernels.
 * Our DESIGN.md requires 6.1 minimum so the shim is a belt-and-braces
 * guard for out-of-tree backports.
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(5, 3, 0)
static inline u64 aivpn_ktime_ms(void)
{
	return ktime_get_ms();
}
#else
static inline u64 aivpn_ktime_ms(void)
{
	return (u64)ktime_to_ms(ktime_get());
}
#endif

/* ------------------------------------------------------------------ *
 *  netif_rx_ni compatibility shim                                      *
 *                                                                      *
 *  netif_rx_ni() was removed in 5.18; callers should use netif_rx()   *
 *  instead (it has been safe to call from any context since 5.18).    *
 * ------------------------------------------------------------------ */

#if LINUX_VERSION_CODE < KERNEL_VERSION(5, 18, 0)
/* On older kernels netif_rx_ni exists; alias so callers use one name. */
#define aivpn_netif_rx(skb)  netif_rx_ni(skb)
#else
#define aivpn_netif_rx(skb)  netif_rx(skb)
#endif

#endif /* AIVPN_HELPERS_H */
