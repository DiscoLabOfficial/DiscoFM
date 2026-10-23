/* Direct upstream trace driver, independent of bridge allocation/chunk logic.
 * This checks integration agreement, NOT independent emulator/hardware fidelity. */
#include <stddef.h>
#include <stdint.h>
#include "ym3438.h"

typedef struct { uint64_t tick; uint32_t port; uint8_t data; } dfm_event;
void dfm_oracle(unsigned type, const dfm_event *events, size_t event_count,
                uint64_t capture_start, size_t capture_ticks, short *output) {
    ym3438_t chip;
    uint64_t tick, end = capture_start + capture_ticks;
    size_t event = 0;
    short pins[2];
    OPN2_SetChipType(type);
    OPN2_Reset(&chip);
    for (tick = 0; tick < end; ++tick) {
        while (event < event_count && events[event].tick == tick) {
            OPN2_Write(&chip, events[event].port, events[event].data);
            ++event;
        }
        OPN2_Clock(&chip, pins);
        if (tick >= capture_start) {
            output[2 * (size_t)(tick - capture_start)] = pins[0];
            output[2 * (size_t)(tick - capture_start) + 1] = pins[1];
        }
    }
}
