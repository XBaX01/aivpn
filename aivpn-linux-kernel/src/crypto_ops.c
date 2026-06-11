// SPDX-License-Identifier: GPL-2.0
/*
 * crypto_ops.c — per-session ChaCha20-Poly1305 AEAD for aivpn.ko
 *
 * Nonce layout (12 bytes):
 *   nonce[0..8]  = counter (little-endian u64)
 *   nonce[8..12] = session->nonce_suffix[0..4]
 *
 * AAD = 8-byte resonance tag.  Auth tag = 16 bytes (Poly1305).
 *
 * CRITICAL: the counter used for the nonce comes from the tag-window lookup
 * (aivpn_tag_lookup), NOT from the wire.  Extracting the counter from the
 * wire would let an attacker force arbitrary nonces and break AEAD security.
 */

#include <linux/slab.h>
#include <linux/scatterlist.h>
#include <linux/skbuff.h>
#include <crypto/aead.h>
#include "crypto_ops.h"
#include "helpers.h"

#define AIVPN_NONCE_SIZE  12
#define AIVPN_AUTH_SIZE   16

static void build_nonce(u8 nonce[AIVPN_NONCE_SIZE],
			u64 counter, const u8 nonce_suffix[4])
{
	__le64 ctr_le = cpu_to_le64(counter);
	memcpy(nonce, &ctr_le, 8);
	memcpy(nonce + 8, nonce_suffix, 4);
}

/* aivpn_decrypt — inbound fast path
 *
 * @s:       session; s->lock held by caller (via spin_lock_bh)
 * @skb:     [8-byte tag][ciphertext][16-byte auth tag]
 * @counter: from tag-window lookup, used to build the AEAD nonce
 *
 * On success: skb->data points to decrypted IP payload.
 * Stats (rx_packets, rx_bytes) updated inside the caller's s->lock.
 */
int aivpn_decrypt(struct aivpn_kern_session *s, struct sk_buff *skb, u64 counter)
{
	struct aead_request *req;
	struct scatterlist sg_data;
	DECLARE_CRYPTO_WAIT(wait);
	u8 nonce[AIVPN_NONCE_SIZE];
	unsigned int data_len;
	int ret;

	if (skb->len < AIVPN_TAG_SIZE + AIVPN_AUTH_SIZE)
		return -EINVAL;

	if (skb_linearize(skb))
		return -ENOMEM;

	/* Nonce from tag-window counter, not from wire bytes */
	build_nonce(nonce, counter, s->nonce_suffix);

	/* User-space encrypt_payload() uses no AAD; assoclen must be 0. */
	data_len = skb->len - AIVPN_TAG_SIZE; /* ciphertext + auth tag */
	sg_init_one(&sg_data, skb->data + AIVPN_TAG_SIZE, data_len);

	req = aead_request_alloc(s->tfm, GFP_ATOMIC);
	if (!req)
		return -ENOMEM;

	aead_request_set_callback(req, CRYPTO_TFM_REQ_MAY_BACKLOG,
				  crypto_req_done, &wait);
	aead_request_set_ad(req, 0);
	aead_request_set_crypt(req, &sg_data, &sg_data, data_len, nonce);

	ret = crypto_wait_req(crypto_aead_decrypt(req), &wait);
	aead_request_free(req);
	if (ret) {
		aivpn_dbg("decrypt failed: %d\n", ret);
		return ret;
	}

	/* Remove tag prefix and auth suffix; expose plaintext */
	skb_pull(skb, AIVPN_TAG_SIZE);
	skb_trim(skb, skb->len - AIVPN_AUTH_SIZE);

	/* Update stats after trim so rx_bytes counts plaintext only */
	s->rx_packets++;
	s->rx_bytes += skb->len;
	return 0;
}

/* aivpn_encrypt — outbound path (kernel-side TX, optional)
 *
 * @s:   session; NOT locked — uses atomic tx_counter
 * @skb: plaintext IP packet
 *
 * Prepends 8-byte resonance tag (LE64 of counter) and appends 16-byte auth tag.
 */
int aivpn_encrypt(struct aivpn_kern_session *s, struct sk_buff *skb)
{
	struct aead_request *req;
	struct scatterlist sg_data;
	DECLARE_CRYPTO_WAIT(wait);
	u8 nonce[AIVPN_NONCE_SIZE];
	u8 tag[AIVPN_TAG_SIZE];
	u64 counter;
	unsigned int plain_len = skb->len;
	int ret;

	if (skb_headroom(skb) < AIVPN_TAG_SIZE ||
	    skb_tailroom(skb) < AIVPN_AUTH_SIZE) {
		if (pskb_expand_head(skb, AIVPN_TAG_SIZE, AIVPN_AUTH_SIZE,
				     GFP_ATOMIC))
			return -ENOMEM;
	}

	counter = (u64)atomic64_inc_return(&s->tx_counter) - 1;
	build_nonce(nonce, counter, s->nonce_suffix);

	{
		__le64 ctr_le = cpu_to_le64(counter);
		memcpy(tag, &ctr_le, AIVPN_TAG_SIZE);
	}

	skb_put(skb, AIVPN_AUTH_SIZE);
	sg_init_one(&sg_data, skb->data, plain_len + AIVPN_AUTH_SIZE);

	req = aead_request_alloc(s->tfm, GFP_ATOMIC);
	if (!req)
		return -ENOMEM;

	aead_request_set_callback(req, CRYPTO_TFM_REQ_MAY_BACKLOG,
				  crypto_req_done, &wait);
	aead_request_set_ad(req, 0); /* user-space uses no AAD */
	aead_request_set_crypt(req, &sg_data, &sg_data, plain_len, nonce);

	ret = crypto_wait_req(crypto_aead_encrypt(req), &wait);
	aead_request_free(req);
	if (ret) {
		aivpn_dbg("encrypt failed: %d\n", ret);
		return ret;
	}

	skb_push(skb, AIVPN_TAG_SIZE);
	memcpy(skb->data, tag, AIVPN_TAG_SIZE);

	spin_lock_bh(&s->lock);
	s->tx_packets++;
	s->tx_bytes += plain_len;
	spin_unlock_bh(&s->lock);
	return 0;
}
