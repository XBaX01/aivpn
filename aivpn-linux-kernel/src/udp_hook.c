// SPDX-License-Identifier: GPL-2.0
/*
 * udp_hook.c — intercept UDP socket RX for in-kernel VPN fast path
 *
 * After AIVPN_IOC_SET_UDP_SOCK, sk->sk_data_ready is replaced with
 * aivpn_sk_data_ready.  Matched packets go: tag lookup → decrypt →
 * tun inject.  Unmatched packets fall back to user space.
 */

#include <linux/net.h>
#include <linux/skbuff.h>
#include <net/sock.h>
#include "udp_hook.h"
#include "session_table.h"
#include "crypto_ops.h"
#include "tun_inject.h"
#include "helpers.h"

struct aivpn_hook_state {
	void (*orig_data_ready)(struct sock *sk);
};

static void aivpn_sk_data_ready(struct sock *sk)
{
	struct aivpn_hook_state *hs = sk->sk_user_data;
	struct sk_buff *skb;
	struct aivpn_kern_session *session;
	u64 now_ms;
	int ret, err;

	while ((skb = skb_recv_datagram(sk, MSG_DONTWAIT, &err)) != NULL) {
		if (skb->len < AIVPN_TAG_SIZE)
			goto fallback;

		now_ms = aivpn_ktime_ms();
		session = aivpn_session_lookup(skb->data, now_ms);
		if (!session)
			goto fallback;

		ret = aivpn_decrypt(session, skb);
		spin_unlock(&session->lock);

		if (ret) {
			kfree_skb(skb);
			continue;
		}

		if (aivpn_tun_inject(skb))
			kfree_skb(skb);
		continue;

fallback:
		skb_queue_tail(&sk->sk_receive_queue, skb);
		if (hs && hs->orig_data_ready)
			hs->orig_data_ready(sk);
	}
}

int aivpn_udp_hook_install(struct socket *sock)
{
	struct sock *sk = sock->sk;
	struct aivpn_hook_state *hs;

	if (!sk)
		return -EINVAL;

	hs = kzalloc(sizeof(*hs), GFP_KERNEL);
	if (!hs)
		return -ENOMEM;

	lock_sock(sk);
	hs->orig_data_ready = sk->sk_data_ready;
	sk->sk_user_data    = hs;
	sk->sk_data_ready   = aivpn_sk_data_ready;
	release_sock(sk);

	aivpn_info("UDP hook installed\n");
	return 0;
}

void aivpn_udp_hook_remove(struct socket *sock)
{
	struct sock *sk;
	struct aivpn_hook_state *hs;

	if (!sock || !sock->sk)
		return;
	sk = sock->sk;

	lock_sock(sk);
	hs = sk->sk_user_data;
	if (hs) {
		sk->sk_data_ready = hs->orig_data_ready;
		sk->sk_user_data  = NULL;
		kfree(hs);
	}
	release_sock(sk);

	aivpn_info("UDP hook removed\n");
}
