/* DiscoFM native boundary. Build-generated core has instance-local chip type.
 * The pristine vendor is compiled separately for the independent oracle. */
#include <stdlib.h>
#include "ym3438.h"

void *dfm_create(void) {
    ym3438_t *chip = (ym3438_t *)malloc(sizeof(ym3438_t));
    if (chip) { DFM_OPN2_Reset(chip); chip->dfm_chip_type = ym3438_mode_readmode; }
    return chip;
}
void dfm_destroy(void *chip) { free(chip); }
void dfm_reset(void *pointer) {
    ym3438_t *chip = (ym3438_t *)pointer;
    Bit32u type = chip->dfm_chip_type;
    DFM_OPN2_Reset(chip);
    chip->dfm_chip_type = type;
}
void dfm_write(void *chip, unsigned type, unsigned port, unsigned char data) {
    ((ym3438_t *)chip)->dfm_chip_type = type;
    DFM_OPN2_Write((ym3438_t *)chip, port, data);
}
unsigned char dfm_status(void *chip, unsigned type) {
    ((ym3438_t *)chip)->dfm_chip_type = type;
    return DFM_OPN2_Read((ym3438_t *)chip, 0);
}
void dfm_clock(void *chip, unsigned type, short *output, size_t ticks) {
    size_t i;
    ((ym3438_t *)chip)->dfm_chip_type = type;
    for (i = 0; i < ticks; ++i) DFM_OPN2_Clock((ym3438_t *)chip, output + 2 * i);
}

/* Read-only diagnostics, without advancing or changing the upstream chip.
 * Contract: 4 operators x 16 words, then 18 channel/global words (82 total).
 * Slots are S1/S2/S3/S4, NOT sequential register or pipeline order. */
void dfm_snapshot(void *pointer, unsigned channel, uint32_t *words) {
    const ym3438_t *chip = (const ym3438_t *)pointer;
    static const unsigned slots[4] = { 0, 12, 6, 18 };
    unsigned i, slot;
    uint32_t *op;
    for (i = 0; i < 4; ++i) {
        slot = channel + slots[i];
        op = words + i * 16;
        op[0] = chip->dt[slot]; op[1] = chip->multi[slot];
        op[2] = chip->tl[slot]; op[3] = chip->ks[slot];
        op[4] = chip->ar[slot]; op[5] = chip->dr[slot];
        op[6] = chip->sr[slot]; op[7] = chip->sl[slot];
        op[8] = chip->rr[slot]; op[9] = chip->am[slot];
        op[10] = chip->ssg_eg[slot]; op[11] = chip->eg_level[slot];
        op[12] = chip->eg_out[slot]; op[13] = chip->eg_state[slot];
        op[14] = chip->pg_inc[slot]; op[15] = chip->pg_phase[slot];
    }
    words[64] = chip->connect[channel]; words[65] = chip->fb[channel];
    words[66] = chip->ams[channel]; words[67] = chip->pms[channel];
    words[68] = chip->pan_l[channel]; words[69] = chip->pan_r[channel];
    words[70] = chip->fnum[channel]; words[71] = chip->block[channel];
    words[72] = chip->lfo_en; words[73] = chip->lfo_freq;
    words[74] = chip->mode_ch3; words[75] = chip->dacen;
    for (i = 0; i < 3; ++i) {
        words[76 + i * 2] = chip->fnum_3ch[i];
        words[77 + i * 2] = chip->block_3ch[i];
    }
}
