// SPDX-License-Identifier: GPL-2.0
/*
 * tun_inject.c — inject decrypted packets into the TUN net_device via netif_rx
 *
 * MEDIUM-7 fix: the PI header (tun_pi flags+proto) is only required when
 * writing to a TUN file descriptor.  Direct netif_rx injection needs only
 * skb->protocol set correctly; no push/pull of a PI header is needed.
 */

#include <linux/netdevice.h>
#include <linux/skbuff.h>
#include <linux/spinlock.h>
#include <net/net_namespace.h>
#include "tun_inject.h"
#include "helpers.h"

static struct net_device *aivpn_tun_dev __read_mostly = NULL;
static DEFINE_SPINLOCK(aivpn_tun_lock);

int aivpn_tun_set_device(u32 ifindex)
{
	struct net_device *dev;

	dev = dev_get_by_index(&init_net, ifindex);
	if (!dev) {
		aivpn_err("tun set_device: ifindex %u not found\n", ifindex);
		return -ENODEV;
	}

	spin_lock_bh(&aivpn_tun_lock);
	if (aivpn_tun_dev)
		dev_put(aivpn_tun_dev);
	aivpn_tun_dev = dev;
	spin_unlock_bh(&aivpn_tun_lock);

	aivpn_info("TUN device set: %s (ifindex %u)\n", dev->name, ifindex);
	return 0;
}

void aivpn_tun_clear(void)
{
	struct net_device *dev;

	spin_lock_bh(&aivpn_tun_lock);
	dev = aivpn_tun_dev;
	aivpn_tun_dev = NULL;
	spin_unlock_bh(&aivpn_tun_lock);

	if (dev)
		dev_put(dev);
}

int aivpn_tun_inject(struct sk_buff *skb)
{
	struct net_device *dev;

	spin_lock_bh(&aivpn_tun_lock);
	dev = aivpn_tun_dev;
	if (dev)
		dev_hold(dev);
	spin_unlock_bh(&aivpn_tun_lock);

	if (!dev)
		return -ENODEV;

	skb->dev      = dev;
	skb->protocol = (skb->len > 0 && (skb->data[0] >> 4) == 6)
			? htons(ETH_P_IPV6) : htons(ETH_P_IP);
	skb_reset_mac_header(skb);
	skb_reset_network_header(skb);

	aivpn_netif_rx(skb);
	dev_put(dev);
	return 0;
}
