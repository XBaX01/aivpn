/* SPDX-License-Identifier: GPL-2.0 */
/*
 * tun_inject.h — TUN device injection helpers for aivpn.ko
 */

#ifndef AIVPN_TUN_INJECT_H
#define AIVPN_TUN_INJECT_H

#include <linux/skbuff.h>

/**
 * aivpn_tun_set_device - register TUN net_device by ifindex.
 * Returns 0 on success, -ENODEV if not found.
 */
int  aivpn_tun_set_device(u32 ifindex);

/**
 * aivpn_tun_inject - deliver decrypted skb into the TUN device.
 * Prepends 4-byte PI header (ETH_P_IP) and calls netif_rx().
 * Returns 0 on success, -ENODEV if no device registered.
 */
int  aivpn_tun_inject(struct sk_buff *skb);

/** Release the net_device reference. */
void aivpn_tun_clear(void);

#endif /* AIVPN_TUN_INJECT_H */
