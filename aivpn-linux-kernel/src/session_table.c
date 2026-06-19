// SPDX-License-Identifier: GPL-2.0
/*
 * session_table.c — two-table RCU session store for aivpn.ko
 *
 * aivpn_tag_htable    (128K RCU buckets) — tag→session, read from BH/softirq
 * aivpn_session_htable  (512 buckets)    — session_id→session, mgmt only
 *
 * Locking discipline:
 *   aivpn_table_lock  DEFINE_SPINLOCK  — write-side for both tables;
 *                                        always entered with spin_lock_bh
 *   session->lock     spinlock         — anti-replay window + stats;
 *                                        entered with spin_lock_bh everywhere
 *                                        (safe from both BH and process context)
 *
 * Tag lookup hot path: rcu_read_lock() → hash_for_each_possible_rcu() → rcu_read_unlock()
 * No sleepable code is called on the RX fast path.
 */

#include <linux/slab.h>
#include <linux/jhash.h>
#include <linux/string.h>
#include <linux/errno.h>
#include <linux/bitmap.h>
#include <linux/rcupdate.h>
#include <linux/atomic.h>
#include <crypto/aead.h>
#include "session_table.h"
#include "helpers.h"

static DEFINE_HASHTABLE(aivpn_tag_htable,     AIVPN_TAG_HASH_BITS);
static DEFINE_HASHTABLE(aivpn_session_htable, AIVPN_SESSION_HASH_BITS);
static DEFINE_SPINLOCK(aivpn_table_lock);
static atomic_t aivpn_session_count = ATOMIC_INIT(0);

/* ── Init / fini ─────────────────────────────────────────────────────────── */

int aivpn_session_table_init(void)
{
	hash_init(aivpn_tag_htable);
	hash_init(aivpn_session_htable);
	aivpn_info("session table ready (tag: %u, session: %u buckets)\n",
		   1u << AIVPN_TAG_HASH_BITS, 1u << AIVPN_SESSION_HASH_BITS);
	return 0;
}

void aivpn_session_table_fini(void)
{
	aivpn_session_flush();
}

/* ── Hash helpers ────────────────────────────────────────────────────────── */

static u32 tag_hash_key(const u8 *tag)
{
	return jhash(tag, AIVPN_TAG_SIZE, 0);
}

static u32 sid_hash_key(const u8 *session_id)
{
	return jhash(session_id, 16, 0);
}

/* ── RCU callback: free a single tag entry ───────────────────────────────── */

static void tag_entry_free_rcu(struct rcu_head *head)
{
	struct aivpn_tag_entry *e = container_of(head, struct aivpn_tag_entry, rcu);
	kfree_sensitive(e);
}

/* ── Session object lifecycle ────────────────────────────────────────────── */

static void session_free(struct aivpn_kern_session *s)
{
	if (s->tfm) {
		crypto_free_aead(s->tfm);
		s->tfm = NULL;
	}
	memzero_explicit(s->nonce_suffix, sizeof(s->nonce_suffix));
	kfree(s);
}

/* ── aivpn_session_insert ────────────────────────────────────────────────── */

int aivpn_session_insert(const struct aivpn_session_add *add)
{
	struct aivpn_kern_session *s;
	struct crypto_aead *tfm;
	int ret;

	s = kzalloc(sizeof(*s), GFP_KERNEL);
	if (!s)
		return -ENOMEM;

	memcpy(s->session_id,   add->session_id,  sizeof(s->session_id));
	memcpy(s->nonce_suffix, add->nonce_suffix, sizeof(s->nonce_suffix));
	s->client_ip = add->client_ip;
	spin_lock_init(&s->lock);
	atomic64_set(&s->tx_counter, (s64)add->counter_base);
	/* recv_counter and replay_window are zero-initialised by kzalloc */

	tfm = crypto_alloc_aead("rfc7539(chacha20,poly1305)", 0, 0);
	if (IS_ERR(tfm)) {
		ret = PTR_ERR(tfm);
		kfree(s);
		return ret;
	}
	ret = crypto_aead_setkey(tfm, add->session_key, 32);
	if (ret) { crypto_free_aead(tfm); kfree(s); return ret; }
	ret = crypto_aead_setauthsize(tfm, 16);
	if (ret) { crypto_free_aead(tfm); kfree(s); return ret; }
	s->tfm = tfm;

	spin_lock_bh(&aivpn_table_lock);
	if (atomic_read(&aivpn_session_count) >= MAX_SESSIONS) {
		spin_unlock_bh(&aivpn_table_lock);
		crypto_free_aead(tfm);
		kfree(s);
		return -ENOSPC;
	}
	hash_add(aivpn_session_htable, &s->mgmt_node, sid_hash_key(s->session_id));
	atomic_inc(&aivpn_session_count);
	spin_unlock_bh(&aivpn_table_lock);
	return 0;
}

/* ── aivpn_session_tags_update ───────────────────────────────────────────── */

int aivpn_session_tags_update(const struct aivpn_session_update_tags *upd)
{
	struct aivpn_kern_session *s, *candidate;
	struct aivpn_tag_entry *new_entries[AIVPN_TAG_WINDOW_SLOTS];
	struct aivpn_tag_entry *old_entries[AIVPN_TAG_WINDOW_SLOTS];
	int old_count = 0;
	u32 count, i;

	count = upd->count;
	if (count > AIVPN_TAG_WINDOW_SLOTS)
		return -EINVAL;

	/* Pre-allocate all new tag entries before acquiring any lock */
	for (i = 0; i < count; i++) {
		new_entries[i] = kzalloc(sizeof(*new_entries[i]), GFP_KERNEL);
		if (!new_entries[i]) {
			while (i--)
				kfree(new_entries[i]);
			return -ENOMEM;
		}
		memcpy(new_entries[i]->tag, upd->entries[i].tag, AIVPN_TAG_SIZE);
		new_entries[i]->counter = upd->entries[i].counter;
	}

	/* Locate the session; hold table lock so it cannot be concurrently removed */
	spin_lock_bh(&aivpn_table_lock);
	s = NULL;
	hash_for_each_possible(aivpn_session_htable, candidate, mgmt_node,
			       sid_hash_key(upd->session_id)) {
		if (!crypto_memneq(candidate->session_id, upd->session_id, 16)) {
			s = candidate;
			break;
		}
	}
	if (!s) {
		spin_unlock_bh(&aivpn_table_lock);
		for (i = 0; i < count; i++)
			kfree(new_entries[i]);
		return -ENOENT;
	}

	/* Unlink old tag entries from the RCU table */
	old_count = s->tag_entry_count;
	for (i = 0; i < (u32)old_count; i++) {
		old_entries[i] = s->tag_entries[i];
		if (old_entries[i])
			hash_del_rcu(&old_entries[i]->hnode);
	}

	/* Link new entries into the RCU table and update session bookkeeping */
	for (i = 0; i < count; i++) {
		new_entries[i]->session = s;
		hash_add_rcu(aivpn_tag_htable, &new_entries[i]->hnode,
			     tag_hash_key(new_entries[i]->tag));
		s->tag_entries[i] = new_entries[i];
	}
	for (i = count; i < (u32)old_count; i++)
		s->tag_entries[i] = NULL;
	s->tag_entry_count = (int)count;
	spin_unlock_bh(&aivpn_table_lock);

	/* Free old entries after an RCU grace period */
	for (i = 0; i < (u32)old_count; i++) {
		if (old_entries[i])
			call_rcu(&old_entries[i]->rcu, tag_entry_free_rcu);
	}
	return 0;
}

/* ── aivpn_tag_lookup — hot path, called with rcu_read_lock() held ─────── */

struct aivpn_kern_session *aivpn_tag_lookup(const u8 *tag, u64 *counter)
{
	struct aivpn_tag_entry *e;
	u32 h = tag_hash_key(tag);

	hash_for_each_possible_rcu(aivpn_tag_htable, e, hnode, h) {
		if (!crypto_memneq(e->tag, tag, AIVPN_TAG_SIZE)) {
			*counter = e->counter;
			return e->session;
		}
	}
	return NULL;
}

/* ── aivpn_counter_validate — WireGuard-style anti-replay ──────────────── */

bool aivpn_counter_validate(struct aivpn_kern_session *s, u64 counter)
{
	u64 diff;
	u32 index;

	/* Caller must hold s->lock (via spin_lock_bh) */

	if (counter > s->recv_counter) {
		/* Advance window; shift left so bit 0 = recv_counter */
		diff = counter - s->recv_counter;
		if (diff < AIVPN_REPLAY_WINDOW) {
			bitmap_shift_left(s->replay_window, s->replay_window,
					  (unsigned int)diff, AIVPN_REPLAY_WINDOW);
		} else {
			bitmap_zero(s->replay_window, AIVPN_REPLAY_WINDOW);
		}
		s->recv_counter = counter;
		set_bit(0, s->replay_window);
		return true;
	}

	diff = s->recv_counter - counter;
	if (diff >= AIVPN_REPLAY_WINDOW)
		return false; /* too old */

	index = (u32)diff;
	if (test_bit(index, s->replay_window))
		return false; /* replay */

	set_bit(index, s->replay_window);
	return true;
}

/* ── aivpn_session_remove ────────────────────────────────────────────────── */

int aivpn_session_remove(const u8 *session_id)
{
	struct aivpn_kern_session *s, *candidate;
	struct aivpn_tag_entry *saved[AIVPN_TAG_WINDOW_SLOTS];
	int saved_count, i;

	spin_lock_bh(&aivpn_table_lock);
	s = NULL;
	hash_for_each_possible(aivpn_session_htable, candidate, mgmt_node,
			       sid_hash_key(session_id)) {
		if (!crypto_memneq(candidate->session_id, session_id, 16)) {
			s = candidate;
			break;
		}
	}
	if (!s) {
		spin_unlock_bh(&aivpn_table_lock);
		return -ENOENT;
	}

	hash_del(&s->mgmt_node);
	atomic_dec(&aivpn_session_count);
	saved_count = s->tag_entry_count;
	for (i = 0; i < saved_count; i++) {
		saved[i] = s->tag_entries[i];
		if (saved[i])
			hash_del_rcu(&saved[i]->hnode);
	}
	spin_unlock_bh(&aivpn_table_lock);

	/* Wait for all in-flight RCU readers before freeing */
	synchronize_rcu();

	/*
	 * After synchronize_rcu() no new data-path thread can obtain a pointer
	 * to this session, but a thread that got the pointer just before we
	 * removed the tag entries may still be holding session->lock (decrypt
	 * in progress).  Acquire and immediately release the lock to drain any
	 * such in-flight operation before freeing.
	 */
	spin_lock_bh(&s->lock);
	spin_unlock_bh(&s->lock);

	for (i = 0; i < saved_count; i++) {
		if (saved[i])
			kfree_sensitive(saved[i]);
	}
	session_free(s);
	return 0;
}

/* ── aivpn_session_flush ─────────────────────────────────────────────────── */

void aivpn_session_flush(void)
{
	struct aivpn_kern_session *s;
	struct hlist_node *tmp;
	HLIST_HEAD(to_free);
	int bkt, i;

	spin_lock_bh(&aivpn_table_lock);
	hash_for_each_safe(aivpn_session_htable, bkt, tmp, s, mgmt_node) {
		hash_del(&s->mgmt_node);
		for (i = 0; i < s->tag_entry_count; i++) {
			if (s->tag_entries[i])
				hash_del_rcu(&s->tag_entries[i]->hnode);
		}
		/* Re-use mgmt_node (removed from htable) to chain the free list */
		hlist_add_head(&s->mgmt_node, &to_free);
	}
	atomic_set(&aivpn_session_count, 0);
	spin_unlock_bh(&aivpn_table_lock);

	synchronize_rcu();

	hlist_for_each_entry_safe(s, tmp, &to_free, mgmt_node) {
		hlist_del(&s->mgmt_node);
		/* Drain any in-progress decrypt before freeing (see session_remove). */
		spin_lock_bh(&s->lock);
		spin_unlock_bh(&s->lock);
		for (i = 0; i < s->tag_entry_count; i++) {
			if (s->tag_entries[i]) {
				kfree_sensitive(s->tag_entries[i]);
				s->tag_entries[i] = NULL;
			}
		}
		session_free(s);
	}
}

/* ── aivpn_session_stat ──────────────────────────────────────────────────── */

int aivpn_session_stat(struct aivpn_session_stat *stat)
{
	struct aivpn_kern_session *s, *candidate;
	u32 h = sid_hash_key(stat->session_id);

	spin_lock_bh(&aivpn_table_lock);
	s = NULL;
	hash_for_each_possible(aivpn_session_htable, candidate, mgmt_node, h) {
		if (!crypto_memneq(candidate->session_id, stat->session_id, 16)) {
			s = candidate;
			break;
		}
	}
	if (!s) {
		spin_unlock_bh(&aivpn_table_lock);
		return -ENOENT;
	}
	spin_lock_bh(&s->lock);
	stat->active     = 1;
	stat->rx_packets = s->rx_packets;
	stat->tx_packets = s->tx_packets;
	stat->rx_bytes   = s->rx_bytes;
	stat->tx_bytes   = s->tx_bytes;
	spin_unlock_bh(&s->lock);
	spin_unlock_bh(&aivpn_table_lock);
	return 0;
}
