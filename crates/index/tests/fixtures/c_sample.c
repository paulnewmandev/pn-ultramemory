// SPDX-License-Identifier: Apache-2.0
/* Fixture for the extraction tests: a tiny ring buffer library in C. */
#include <stdio.h>
#include <stdlib.h>
#include "ring.h"

#define RING_CAPACITY 64
#define RING_MASK(x) ((x) & (RING_CAPACITY - 1))
#define INCLUDE_GUARD_H

/** A fixed-size ring buffer. */
typedef struct Ring {
    int items[RING_CAPACITY];
    size_t head;
    size_t tail;
} Ring;

struct Node {
    int value;
    struct Node *next;
};

/** States of a ring. */
enum RingState { RING_EMPTY, RING_FULL };

typedef enum { MODE_READ, MODE_WRITE } Mode;

typedef int (*visitor_fn)(int);

union Cell {
    int i;
    float f;
};

static int allocated = 0;
int global_counter;
extern int shared_flag;
const int LIMIT = 10;

int ring_push(Ring *ring, int value);
static void log_event(const char *msg);

/**
 * Creates a ring.
 * Returns NULL on failure.
 */
Ring *ring_new(void)
{
    Ring *ring = malloc(sizeof(Ring));
    if (ring == NULL) {
        log_event("alloc failed");
        return NULL;
    }
    ring->head = ring->tail = 0;
    allocated++;
    return ring;
}

/** Pushes a value. */
int ring_push(Ring *ring, int value)
{
    size_t next = RING_MASK(ring->head + 1);
    if (next == ring->tail) {
        return -1;
    }
    ring->items[ring->head] = value;
    ring->head = next;
    return 0;
}

static void log_event(const char *msg)
{
    fprintf(stderr, "ring: %s\n", msg);
    flush_all();
}

int main(int argc, char **argv)
{
    Ring *ring = ring_new();
    ring_push(ring, argc);
    return 0;
}
