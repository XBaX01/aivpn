/* SPDX-License-Identifier: GPL-2.0 */
/*
 * udp_hook.h — UDP socket sk_data_ready intercept for aivpn.ko
 */

#ifndef AIVPN_UDP_HOOK_H
#define AIVPN_UDP_HOOK_H

#include <linux/net.h>

/**
 * aivpn_udp_hook_install - replace sk_data_ready on the given socket.
 * @sock: the UDP socket whose RX the module should intercept.
 *
 * Saves the original sk_data_ready and installs aivpn_sk_data_ready.
 * Returns 0 on success, -errno on error.
 */
int  aivpn_udp_hook_install(struct socket *sock);

/**
 * aivpn_udp_hook_remove - restore the original sk_data_ready callback.
 * @sock: the socket passed to aivpn_udp_hook_install().
 */
void aivpn_udp_hook_remove(struct socket *sock);

#endif /* AIVPN_UDP_HOOK_H */
