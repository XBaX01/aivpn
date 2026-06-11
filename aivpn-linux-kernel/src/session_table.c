// SPDX-License-Identifier: GPL-2.0
/*
 * session_table.c — DEFINE_HASHTABLE-based session store for aivpn.ko
 *
 * 512-bucket hash map keyed on the 8-byte resonance tag.
 * Global rwlock_t gates structural changes; per-session spinlock_t
 * protects the anti-replay bitmap.
 */

#include <linux/slab.h>
#include <linux/rwlock.h>
#include <linux/jhash.h>
#include <linux/string.h>
#include <linux/errno.h>
#include <linux/byteorder/generic.h>
#include <crypto/aead.h>
#include "session_table.h"
#include "helpers.h"

static DEFINE_HASHTABLE(aivpn_htable, AIVPN_HASH_BITS);
static DEFINE_RWLOCK(aivpn_table_lock);
static atomic_t aivpn_session_count = ATOMIC_INIT(0);

int aivpn_session_table_init(void)
{
	hash_init(aivpn_htable);
	aivpn_info("session table initialised (%u buckets)\n",
		   1u << AIVPN_HASH_BITS);
	return 0;
}

void aivpn_session_table_fini(void)
{
	aivpn_session_flush();
}

static u32 tag_hash(const u8 *tag)
{
	return jhash(tag, AIVPN_TAG_SIZE, 0);
}

static void session_free(struct aivpn_kern_session *s)
{
	if (s->tfm) {
		crypto_free_aead(s->tfm);
		s->tfm = NULL;
	}
	memzero_explicit(s->session_key, sizeof(s->session_key));
	memzero_explicit(s->tag_secret,  sizeof(s->tag_secret));
	memzero_explicit(s->prng_seed,   sizeof(s->prng_seed));
	kfree(s);
}

int aivpn_session_insert(const struct aivpn_session_add *add)
{
	struct aivpn_kern_session *s;
	struct crypto_aead *tfm;
	int ret;

	if (atomic_read(&aivpn_session_count) >= MAX_SESSIONS)
		return -ENOSPC;

	s = kzalloc(sizeof(*s), GFP_KERNEL);
	if (!s)
		return -ENOMEM;

	memcpy(s->session_id,  add->session_id,  sizeof(s->session_id));
	memcpy(s->session_key, add->session_key, sizeof(s->session_key));
	memcpy(s->tag_secret,  add->tag_secret,  sizeof(s->tag_secret));
	memcpy(s->prng_seed,   add->prng_seed,   sizeof(s->prng_seed));
	atomic64_set(&s->counter, (s64)add->counter_base);
	s->client_ip = add->client_ip;
	memcpy(&s->client_addr, add->client_addr, sizeof(s->client_addr));
	s->window_ms = add->window_ms ? add->window_ms : 10000ULL;
	spin_lock_init(&s->lock);
	/* Initial tag derived from first 8 bytes of tag_secret */
	memcpy(s->tag, add->tag_secret, AIVPN_TAG_SIZE);

	tfm = crypto_alloc_aead("rfc7539(chacha20,poly1305)", 0, 0);
	if (IS_ERR(tfm)) {
		ret = PTR_ERR(tfm);
		kfree(s);
		return ret;
	}
	ret = crypto_aead_setkey(tfm, s->session_key, sizeof(s->session_key));
	if (ret) { crypto_free_aead(tfm); kfree(s); return ret; }
	ret = crypto_aead_setauthsize(tfm, 16);
	if (ret) { crypto_free_aead(tfm); kfree(s); return ret; }
	s->tfm = tfm;

	write_lock(&aivpn_table_lock);
	hash_add(aivpn_htable, &s->hnode, tag_hash(s->tag));
	atomic_inc(&aivpn_session_count);
	write_unlock(&aivpn_table_lock);
	return 0;
}

struct aivpn_kern_session *aivpn_session_lookup(const u8 *tag, u64 now_ms)
{
	struct aivpn_kern_session *s;
	u32 key = tag_hash(tag);

	read_lock(&aivpn_table_lock);
	hash_for_each_possible(aivpn_htable, s, hnode, key) {
		if (crypto_memneq(s->tag, tag, AIVPN_TAG_SIZE))
			continue;
		spin_lock(&s->lock);
		if (!aivpn_tag_valid_window(s, tag, now_ms)) {
			spin_unlock(&s->lock);
			continue;
		}
		read_unlock(&aivpn_table_lock);
		return s; /* caller holds s->lock */
	}
	read_unlock(&aivpn_table_lock);
	return NULL;
}

int aivpn_session_remove(const u8 *session_id)
{
	struct aivpn_kern_session *s;
	int bkt;

	write_lock(&aivpn_table_lock);
	hash_for_each(aivpn_htable, bkt, s, hnode) {
		if (crypto_memneq(s->session_id, session_id, 16))
			continue;
		hash_del(&s->hnode);
		atomic_dec(&aivpn_session_count);
		write_unlock(&aivpn_table_lock);
		session_free(s);
		return 0;
	}
	write_unlock(&aivpn_table_lock);
	return -ENOENT;
}

void aivpn_session_flush(void)
{
	struct aivpn_kern_session *s;
	struct hlist_node *tmp;
	int bkt;

	write_lock(&aivpn_table_lock);
	hash_for_each_safe(aivpn_htable, bkt, tmp, s, hnode) {
		hash_del(&s->hnode);
		session_free(s);
	}
	atomic_set(&aivpn_session_count, 0);
	write_unlock(&aivpn_table_lock);
}

int aivpn_session_stat(struct aivpn_session_stat *stat)
{
	struct aivpn_kern_session *s;
	int bkt;

	read_lock(&aivpn_table_lock);
	hash_for_each(aivpn_htable, bkt, s, hnode) {
		if (crypto_memneq(s->session_id, stat->session_id, 16))
			continue;
		stat->active     = 1;
		stat->rx_packets = (u64)atomic64_read(&s->rx_packets);
		stat->tx_packets = (u64)atomic64_read(&s->tx_packets);
		stat->rx_bytes   = (u64)atomic64_read(&s->rx_bytes);
		stat->tx_bytes   = (u64)atomic64_read(&s->tx_bytes);
		read_unlock(&aivpn_table_lock);
		return 0;
	}
	read_unlock(&aivpn_table_lock);
	return -ENOENT;
}

bool aivpn_tag_valid_window(struct aivpn_kern_session *s,
			    const u8 *tag, u64 now_ms)
{
	u64 tag_ts;
	unsigned int slot;

	memcpy(&tag_ts, tag, sizeof(tag_ts));
	tag_ts = le64_to_cpu(tag_ts);

	if (now_ms < tag_ts || (now_ms - tag_ts) > s->window_ms)
		return false;

	slot = (unsigned int)((tag_ts >> 6) & 255u);
	return !test_and_set_bit(slot, s->replay_bitmap);
}
