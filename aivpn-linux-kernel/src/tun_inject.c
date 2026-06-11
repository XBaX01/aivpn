// SPDX-License-Identifier: GPL-2.0
/*
 * tun_inject.c — inject decrypted packets into the TUN net_device
 */

#include <linux/netdevice.h>
#include <linux/if_tun.h>
#include <linux/skbuff.h>
#include <linux/spinlock.h>
#include <net/net_namespace.h>
#include "tun_inject.h"
#include "helpers.h"

#define TUN_PI_SIZE  4  /* flags(2) + proto(2) */

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

	spin_lock(&aivpn_tun_lock);
	if (aivpn_tun_dev)
		dev_put(aivpn_tun_dev);
	aivpn_tun_dev = dev;
	spin_unlock(&aivpn_tun_lock);

	aivpn_info("TUN device set: %s (ifindex %u)\n", dev->name, ifindex);
	return 0;
}

void aivpn_tun_clear(void)
{
	struct net_device *dev;

	spin_lock(&aivpn_tun_lock);
	dev = aivpn_tun_dev;
	aivpn_tun_dev = NULL;
	spin_unlock(&aivpn_tun_lock);

	if (dev)
		dev_put(dev);
}

int aivpn_tun_inject(struct sk_buff *skb)
{
	struct net_device *dev;
	struct tun_pi *pi;

	spin_lock(&aivpn_tun_lock);
	dev = aivpn_tun_dev;
	if (dev)
		dev_hold(dev);
	spin_unlock(&aivpn_tun_lock);

	if (!dev)
		return -ENODEV;

	if (skb_headroom(skb) < TUN_PI_SIZE &&
	    pskb_expand_head(skb, TUN_PI_SIZE, 0, GFP_ATOMIC)) {
		dev_put(dev);
		return -ENOMEM;
	}

	pi = (struct tun_pi *)skb_push(skb, TUN_PI_SIZE);
	pi->flags = 0;
	pi->proto = htons(ETH_P_IP);

	skb->dev      = dev;
	skb->protocol = htons(ETH_P_IP);
	skb_reset_mac_header(skb);
	skb_pull(skb, TUN_PI_SIZE);
	skb_reset_network_header(skb);
	skb_push(skb, TUN_PI_SIZE);

	aivpn_netif_rx(skb);
	dev_put(dev);
	return 0;
}
