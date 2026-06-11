/* SPDX-License-Identifier: GPL-2.0 */
/*
 * session_table.h — kernel-side session table for aivpn.ko
 *
 * Two hash tables:
 *   aivpn_tag_htable    — keyed by 8-byte resonance tag (RCU, read from softirq)
 *   aivpn_session_htable — keyed by 16-byte session_id (spinlock, management only)
 *
 * Anti-replay uses a WireGuard-style 256-bit sliding window per session.
 */

#ifndef AIVPN_SESSION_TABLE_H
#define AIVPN_SESSION_TABLE_H

#include <linux/types.h>
#include <linux/hashtable.h>
#include <linux/spinlock.h>
#include <linux/rcupdate.h>
#include <linux/atomic.h>
#include <crypto/aead.h>
#include "../include/uapi/aivpn.h"

/* Hash table sizes */
#define AIVPN_TAG_HASH_BITS      17   /* 128K buckets for tag entries */
#define AIVPN_SESSION_HASH_BITS   9   /* 512 buckets for session objects */

/* WireGuard-style sliding window: 256 bits */
#define AIVPN_REPLAY_WINDOW  256UL

/**
 * struct aivpn_tag_entry - one (tag, counter) entry in the per-session tag window.
 *
 * Installed by AIVPN_IOC_SESSION_UPDATE_TAGS.  The kernel looks up incoming
 * packets by their 8-byte resonance tag; matching an entry gives both the
 * session pointer and the counter needed to build the AEAD nonce.
 *
 * Protected by RCU on the read side (softirq); spinlock_bh on writes.
 */
struct aivpn_tag_entry {
	u8                       tag[8];      /* wire resonance tag (hash key) */
	u64                      counter;     /* counter that generated this tag */
	struct aivpn_kern_session *session;   /* owning session (non-owning ptr) */
	struct hlist_node        hnode;       /* aivpn_tag_htable linkage */
	struct rcu_head          rcu;         /* for call_rcu on removal */
};

/**
 * struct aivpn_kern_session - per-VPN-session state
 *
 * Allocated by AIVPN_IOC_SESSION_ADD, freed after all RCU grace periods
 * following AIVPN_IOC_SESSION_DEL / flush.
 */
struct aivpn_kern_session {
	/* management hash table linkage — keyed on session_id */
	struct hlist_node  mgmt_node;

	/* identity */
	u8   session_id[16];

	/* crypto material — zeroed on removal */
	struct crypto_aead *tfm;      /* ChaCha20-Poly1305, key pre-loaded */
	u8   nonce_suffix[4];         /* bytes 8-11 of the 12-byte nonce */

	/* VPN routing */
	u32  client_ip;               /* VPN IPv4 (network byte order) */

	/* WireGuard-style anti-replay */
	spinlock_t        lock;            /* protects counter + replay_window + stats */
	u64               recv_counter;   /* highest validated counter */
	unsigned long     replay_window[AIVPN_REPLAY_WINDOW / BITS_PER_LONG];

	/* stats (updated under lock) */
	u64  rx_packets;
	u64  rx_bytes;
	u64  tx_packets;
	u64  tx_bytes;

	/* tag window: pointers to tag entries installed in aivpn_tag_htable */
	struct aivpn_tag_entry   *tag_entries[AIVPN_TAG_WINDOW_SLOTS];
	int                       tag_entry_count;

	/* outbound TX counter for aivpn_encrypt(); atomic, no lock needed */
	atomic64_t                tx_counter;
};

/* ── Lifecycle ───────────────────────────────────────────────────────────── */

int  aivpn_session_table_init(void);
void aivpn_session_table_fini(void);

/* ── CRUD ────────────────────────────────────────────────────────────────── */

int  aivpn_session_insert(const struct aivpn_session_add *add);
int  aivpn_session_tags_update(const struct aivpn_session_update_tags *upd);
int  aivpn_session_remove(const u8 *session_id);
void aivpn_session_flush(void);
int  aivpn_session_stat(struct aivpn_session_stat *stat);

/**
 * aivpn_tag_lookup - find a session by its wire resonance tag.
 *
 * Called from softirq.  Returns a pointer with rcu_read_lock held by the
 * caller; fills *counter with the nonce counter for this tag.  Returns NULL
 * if the tag is not in the table.
 *
 * Caller must hold rcu_read_lock() across the returned pointer's use and
 * call rcu_read_unlock() when done.
 */
struct aivpn_kern_session *aivpn_tag_lookup(const u8 *tag, u64 *counter);

/**
 * aivpn_counter_validate - WireGuard-style sliding window anti-replay check.
 *
 * Must be called with session->lock held.
 * Returns true if counter is valid (not a replay); advances the window.
 */
bool aivpn_counter_validate(struct aivpn_kern_session *s, u64 counter);

#endif /* AIVPN_SESSION_TABLE_H */
