// SPDX-License-Identifier: GPL-2.0
/*
 * udp_hook.c — intercept UDP socket RX for in-kernel VPN fast path
 *
 * After AIVPN_IOC_SET_UDP_SOCK, sk->sk_data_ready is replaced with
 * aivpn_sk_data_ready.  On wakeup we atomically drain the receive queue,
 * process recognised packets (tag lookup → anti-replay → decrypt → TUN
 * inject) and return unrecognised packets to the queue so user-space sees them.
 *
 * HIGH-2 fix: we do NOT use skb_recv_datagram for fallback — that dequeues
 * unconditionally.  Instead we splice the entire receive queue into a local
 * list, categorise in-process, then prepend any fallback packets back to the
 * head of sk_receive_queue (before new arrivals) before waking user-space.
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
	struct sk_buff_head process_q, fallback_q;
	struct sk_buff *skb;

	skb_queue_head_init(&process_q);
	skb_queue_head_init(&fallback_q);

	/* Atomically splice the entire receive queue into our local list */
	spin_lock(&sk->sk_receive_queue.lock);
	skb_queue_splice_init(&sk->sk_receive_queue, &process_q);
	spin_unlock(&sk->sk_receive_queue.lock);

	while ((skb = __skb_dequeue(&process_q)) != NULL) {
		struct aivpn_kern_session *session;
		u64 counter;
		int ret;

		if (skb->len < AIVPN_TAG_SIZE) {
			__skb_queue_tail(&fallback_q, skb);
			continue;
		}

		rcu_read_lock();
		session = aivpn_tag_lookup(skb->data, &counter);
		if (!session) {
			rcu_read_unlock();
			__skb_queue_tail(&fallback_q, skb);
			continue;
		}
		/*
		 * Acquire session->lock while still inside rcu_read_lock so that
		 * aivpn_session_remove's synchronize_rcu() cannot complete — and
		 * therefore cannot free the session — until we release rcu_read_lock.
		 * After acquiring the spinlock, rcu_read_unlock is safe: the session
		 * remove path calls spin_lock_bh(s->lock) after synchronize_rcu()
		 * before freeing, so it will wait for us to unlock.
		 */
		spin_lock_bh(&session->lock);
		rcu_read_unlock();

		if (!aivpn_counter_validate(session, counter)) {
			spin_unlock_bh(&session->lock);
			kfree_skb(skb);
			continue;
		}

		ret = aivpn_decrypt(session, skb, counter);
		spin_unlock_bh(&session->lock);

		if (ret) {
			kfree_skb(skb);
			continue;
		}

		if (aivpn_tun_inject(skb))
			kfree_skb(skb);
	}

	/*
	 * Return fallback packets to the head of the receive queue so they
	 * appear before any new packets that arrived while we were processing.
	 * Then wake user-space once.
	 */
	if (!skb_queue_empty(&fallback_q)) {
		spin_lock(&sk->sk_receive_queue.lock);
		skb_queue_splice(&fallback_q, &sk->sk_receive_queue);
		spin_unlock(&sk->sk_receive_queue.lock);

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
