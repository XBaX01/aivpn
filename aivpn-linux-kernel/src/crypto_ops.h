/* SPDX-License-Identifier: GPL-2.0 */
/*
 * crypto_ops.h — ChaCha20-Poly1305 AEAD wrappers for aivpn.ko
 */

#ifndef AIVPN_CRYPTO_OPS_H
#define AIVPN_CRYPTO_OPS_H

#include <linux/skbuff.h>
#include "session_table.h"

/**
 * aivpn_decrypt - decrypt an inbound skb in-place.
 * @s:       session (s->lock held by caller with BH disabled).
 * @skb:     data starts at the 8-byte resonance tag, followed by ciphertext
 *           + 16-byte Poly1305 auth tag.
 * @counter: counter from the tag-window lookup (NOT extracted from wire bytes).
 *
 * On success skb->data points to plaintext IP payload.
 * Returns 0 on success, -errno on failure.
 */
int aivpn_decrypt(struct aivpn_kern_session *s, struct sk_buff *skb, u64 counter);

/**
 * aivpn_encrypt - encrypt an outbound skb in-place.
 * @s:   session (spinlock held by caller).
 * @skb: plaintext IP packet; caller ensures headroom >= 8 and
 *       tailroom >= 16 bytes.
 *
 * Prepends 8-byte resonance tag and appends 16-byte auth tag.
 * Returns 0 on success, -errno on failure.
 */
int aivpn_encrypt(struct aivpn_kern_session *s, struct sk_buff *skb);

#endif /* AIVPN_CRYPTO_OPS_H */
