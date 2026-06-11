/* SPDX-License-Identifier: GPL-2.0 */
/*
 * session_table.h — kernel-side session table for aivpn.ko
 *
 * Wraps a DEFINE_HASHTABLE with 512 buckets keyed on the 8-byte resonance
 * tag.  A global rwlock_t serialises structural changes; per-session
 * spinlock_t protects counter and stats.
 */

#ifndef AIVPN_SESSION_TABLE_H
#define AIVPN_SESSION_TABLE_H

#include <linux/types.h>
#include <linux/hashtable.h>
#include <linux/spinlock.h>
#include <linux/atomic.h>
#include <linux/net.h>
#include <crypto/aead.h>
#include "../include/uapi/aivpn.h"

/* anti-replay bitmap width (256 bits = 4 × unsigned long on 64-bit) */
#define AIVPN_REPLAY_BITS  256
#define AIVPN_REPLAY_WORDS (AIVPN_REPLAY_BITS / BITS_PER_LONG)

/**
 * struct aivpn_kern_session - per-session state in the kernel
 *
 * Lifetime: allocated by aivpn_session_insert(), freed by
 * aivpn_session_remove() / aivpn_session_flush().  Always heap-allocated
 * with kzalloc so fields are zero-initialised before use.
 */
struct aivpn_kern_session {
	/* hash table linkage — keyed on tag[] */
	struct hlist_node  hnode;

	/* identity */
	u8   tag[8];           /* current resonance tag (hash key) */
	u8   session_id[16];   /* opaque identifier from user space */

	/* crypto material — zeroed on removal */
	u8   session_key[32];
	u8   tag_secret[32];
	u8   prng_seed[32];

	/* per-packet counter (atomic for lock-free increment on fast path) */
	atomic64_t counter;

	/* AEAD transform: one handle per session; key pre-loaded */
	struct crypto_aead *tfm;

	/* routing */
	u32  client_ip;                      /* VPN IPv4 (network byte order) */
	struct sockaddr_storage client_addr; /* UDP peer */

	/* tag validity window */
	u64  window_ms;

	/* anti-replay bitmap (256 slots) */
	unsigned long replay_bitmap[AIVPN_REPLAY_WORDS];

	/* stats */
	atomic64_t rx_packets;
	atomic64_t tx_packets;
	atomic64_t rx_bytes;
	atomic64_t tx_bytes;

	/* protects replay_bitmap */
	spinlock_t lock;
};

/* ── Lifecycle ───────────────────────────────────────────────────────────── */

int  aivpn_session_table_init(void);
void aivpn_session_table_fini(void);

/* ── CRUD ────────────────────────────────────────────────────────────────── */

int aivpn_session_insert(const struct aivpn_session_add *add);

/* Returns session with spinlock held; caller must spin_unlock(&s->lock). */
struct aivpn_kern_session *aivpn_session_lookup(const u8 *tag, u64 now_ms);

int  aivpn_session_remove(const u8 *session_id);
void aivpn_session_flush(void);
int  aivpn_session_stat(struct aivpn_session_stat *stat);

/* ── Tag validation ──────────────────────────────────────────────────────── */

bool aivpn_tag_valid_window(struct aivpn_kern_session *s,
			    const u8 *tag, u64 now_ms);

#endif /* AIVPN_SESSION_TABLE_H */
