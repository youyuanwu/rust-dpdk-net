#include "wrapper.h"
#include <rte_errno.h>

int rust_get_rte_errno(void) {
    return rte_errno;
}

void rust_set_rte_errno(int err) {
    rte_errno = err;
}

// Mbuf wrapper implementations
struct rte_mbuf* rust_pktmbuf_alloc(struct rte_mempool *mp) {
    return rte_pktmbuf_alloc(mp);
}

void rust_pktmbuf_free(struct rte_mbuf *m) {
    rte_pktmbuf_free(m);
}

char* rust_pktmbuf_mtod(struct rte_mbuf *m) {
    return rte_pktmbuf_mtod(m, char*);
}

uint16_t rust_pktmbuf_data_len(const struct rte_mbuf *m) {
    return rte_pktmbuf_data_len(m);
}

void rust_pktmbuf_set_data_len(struct rte_mbuf *m, uint16_t len) {
    m->data_len = len;
}

uint32_t rust_pktmbuf_pkt_len(const struct rte_mbuf *m) {
    return rte_pktmbuf_pkt_len(m);
}

void rust_pktmbuf_set_pkt_len(struct rte_mbuf *m, uint32_t len) {
    m->pkt_len = len;
}

uint16_t rust_pktmbuf_headroom(const struct rte_mbuf *m) {
    return rte_pktmbuf_headroom(m);
}

uint16_t rust_pktmbuf_tailroom(const struct rte_mbuf *m) {
    return rte_pktmbuf_tailroom(m);
}

char* rust_pktmbuf_append(struct rte_mbuf *m, uint16_t len) {
    return rte_pktmbuf_append(m, len);
}

char* rust_pktmbuf_prepend(struct rte_mbuf *m, uint16_t len) {
    return rte_pktmbuf_prepend(m, len);
}

char* rust_pktmbuf_adj(struct rte_mbuf *m, uint16_t len) {
    return rte_pktmbuf_adj(m, len);
}

int rust_pktmbuf_trim(struct rte_mbuf *m, uint16_t len) {
    return rte_pktmbuf_trim(m, len);
}

void rust_pktmbuf_reset(struct rte_mbuf *m) {
    rte_pktmbuf_reset(m);
}

uint16_t rust_pktmbuf_data_room_size(struct rte_mempool *mp) {
    return rte_pktmbuf_data_room_size(mp);
}

uint16_t rust_eth_rx_burst(uint16_t port_id, uint16_t queue_id,
                           struct rte_mbuf **rx_pkts, uint16_t nb_pkts) {
    return rte_eth_rx_burst(port_id, queue_id, rx_pkts, nb_pkts);
}

uint16_t rust_eth_tx_burst(uint16_t port_id, uint16_t queue_id,
                           struct rte_mbuf **tx_pkts, uint16_t nb_pkts) {
    return rte_eth_tx_burst(port_id, queue_id, tx_pkts, nb_pkts);
}

// Lcore wrapper implementations
unsigned rust_rte_lcore_id(void) {
    return rte_lcore_id();
}

unsigned rust_rte_get_main_lcore(void) {
    return rte_get_main_lcore();
}
