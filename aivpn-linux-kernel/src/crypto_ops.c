// SPDX-License-Identifier: GPL-2.0
/*
 * crypto_ops.c — per-session ChaCha20-Poly1305 encrypt/decrypt for aivpn.ko
 *
 * Nonce layout (12 bytes, matching user-space encrypt_payload):
 *   nonce[0..8]  = send_counter (little-endian u64)
 *   nonce[8..12] = prng_seed[0..4]
 *
 * AAD = the 8-byte resonance tag.
 * Auth tag size = 16 bytes (Poly1305).
 */

#include <linux/slab.h>
#include <linux/scatterlist.h>
#include <linux/skbuff.h>
#include <crypto/aead.h>
#include "crypto_ops.h"
#include "helpers.h"

#define AIVPN_NONCE_SIZE   12
#define AIVPN_AUTH_SIZE    16
#define AIVPN_COUNTER_OFF  8  /* counter occupies nonce[0..8] */

static void build_nonce(u8 nonce[AIVPN_NONCE_SIZE],
			u64 counter, const u8 *prng_seed)
{
	__le64 ctr_le = cpu_to_le64(counter);
	memcpy(nonce, &ctr_le, AIVPN_COUNTER_OFF);
	memcpy(nonce + AIVPN_COUNTER_OFF, prng_seed, 4);
}

int aivpn_decrypt(struct aivpn_kern_session *s, struct sk_buff *skb)
{
	struct aead_request *req;
	struct scatterlist sg_aad, sg_data;
	DECLARE_CRYPTO_WAIT(wait);
	u8 nonce[AIVPN_NONCE_SIZE];
	u64 counter;
	unsigned int data_len;
	int ret;

	/* Layout: [8-byte tag][ciphertext][16-byte auth tag] */
	if (skb->len < AIVPN_TAG_SIZE + AIVPN_AUTH_SIZE)
		return -EINVAL;

	if (skb_linearize(skb))
		return -ENOMEM;

	/* Extract counter from tag (first 8 bytes, little-endian) */
	{
		__le64 ctr_le;
		memcpy(&ctr_le, skb->data, sizeof(ctr_le));
		counter = le64_to_cpu(ctr_le);
	}
	build_nonce(nonce, counter, s->prng_seed);

	/* AAD = tag bytes; data = ciphertext + auth tag */
	data_len = skb->len - AIVPN_TAG_SIZE;
	sg_init_one(&sg_aad, skb->data, AIVPN_TAG_SIZE);
	sg_init_one(&sg_data, skb->data + AIVPN_TAG_SIZE, data_len);

	req = aead_request_alloc(s->tfm, GFP_ATOMIC);
	if (!req)
		return -ENOMEM;

	aead_request_set_callback(req, CRYPTO_TFM_REQ_MAY_BACKLOG,
				  crypto_req_done, &wait);
	aead_request_set_ad(req, AIVPN_TAG_SIZE);
	aead_request_set_crypt(req, &sg_data, &sg_data, data_len, nonce);

	ret = crypto_wait_req(crypto_aead_decrypt(req), &wait);
	aead_request_free(req);
	if (ret) {
		aivpn_dbg("decrypt failed: %d\n", ret);
		return ret;
	}

	/* Strip tag prefix and auth suffix, expose plaintext */
	skb_pull(skb, AIVPN_TAG_SIZE);
	skb_trim(skb, skb->len - AIVPN_AUTH_SIZE);

	atomic64_inc(&s->rx_packets);
	atomic64_add(skb->len, &s->rx_bytes);
	return 0;
}

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

	counter = (u64)atomic64_inc_return(&s->counter) - 1;
	build_nonce(nonce, counter, s->prng_seed);

	{
		__le64 ctr_le = cpu_to_le64(counter);
		memcpy(tag, &ctr_le, AIVPN_TAG_SIZE);
	}

	/* Extend for auth tag */
	skb_put(skb, AIVPN_AUTH_SIZE);
	sg_init_one(&sg_data, skb->data, plain_len + AIVPN_AUTH_SIZE);

	req = aead_request_alloc(s->tfm, GFP_ATOMIC);
	if (!req)
		return -ENOMEM;

	aead_request_set_callback(req, CRYPTO_TFM_REQ_MAY_BACKLOG,
				  crypto_req_done, &wait);
	aead_request_set_ad(req, AIVPN_TAG_SIZE);
	aead_request_set_crypt(req, &sg_data, &sg_data, plain_len, nonce);

	ret = crypto_wait_req(crypto_aead_encrypt(req), &wait);
	aead_request_free(req);
	if (ret) {
		aivpn_dbg("encrypt failed: %d\n", ret);
		return ret;
	}

	skb_push(skb, AIVPN_TAG_SIZE);
	memcpy(skb->data, tag, AIVPN_TAG_SIZE);

	atomic64_inc(&s->tx_packets);
	atomic64_add(plain_len, &s->tx_bytes);
	return 0;
}
