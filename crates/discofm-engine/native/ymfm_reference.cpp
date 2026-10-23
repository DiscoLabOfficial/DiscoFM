// Independent reference renderer, never used by playback or candidate synthesis.
#include <cstddef>
#include <cstdint>
#include "ymfm_opn.h"

struct DfmPortEvent { uint64_t tick; uint32_t port; uint8_t data; };
template<class Chip>
static void render(const DfmPortEvent *events, size_t count, uint64_t start,
                   size_t frames, int32_t *output) {
    ymfm::ymfm_interface interface;
    Chip chip(interface);
    chip.reset();
    size_t event = 0;
    const uint64_t begin = start / 24;
    for (uint64_t frame = 0; frame < begin + frames; ++frame) {
        // ymfm is a sample-level model. Bus writes take effect at the next
        // 144-master-clock boundary, preserving order (at most one frame late).
        while (event < count && events[event].tick <= frame * 24) {
            chip.write(events[event].port, events[event].data);
            ++event;
        }
        typename Chip::output_data sample;
        chip.generate(&sample);
        if (frame >= begin) {
            output[2 * (frame - begin)] = sample.data[0];
            output[2 * (frame - begin) + 1] = sample.data[1];
        }
    }
}
extern "C" int dfm_ymfm_render(unsigned mode, const DfmPortEvent *events, size_t count,
                               uint64_t start, size_t frames, int32_t *output) noexcept {
    try {
        if (mode == 1) render<ymfm::ym2612>(events, count, start, frames, output);
        else render<ymfm::ym3438>(events, count, start, frames, output);
        return 1;
    } catch (...) { return 0; }
}
