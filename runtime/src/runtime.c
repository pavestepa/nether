#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <stdlib.h>
#include <unistd.h>

typedef struct {
    uint32_t active, kind;
    const char *file;
    uint32_t line, column;
    uint64_t message_len;
    unsigned char message[192];
} NetherPanic;
_Static_assert(sizeof(void *) == 8, "Nether runtime requires a 64-bit target");
_Static_assert(offsetof(NetherPanic, message) == 32, "panic ABI mismatch");
_Static_assert(sizeof(NetherPanic) == 224, "panic ABI mismatch");

static void output(const char *bytes, size_t length) {
    while (length) {
        ssize_t count = write(2, bytes, length);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return;
        bytes += count;
        length -= (size_t)count;
    }
}
static void text(const char *value) {
    size_t length = 0;
    while (value[length]) ++length;
    output(value, length);
}
static void number(uint64_t value) {
    char buffer[20];
    size_t at = sizeof(buffer);
    do { buffer[--at] = (char)('0' + value % 10); value /= 10; } while (value);
    output(buffer + at, sizeof(buffer) - at);
}
void __nether_v1_report_panic(const NetherPanic *panic) {
    static const char *const names[] = {"unknown", "user", "bounds", "overflow", "division", "shift", "allocation"};
    text("Nether panic[");
    text(panic->kind < 7 ? names[panic->kind] : names[0]);
    text("]");
    if (panic->file) {
        text(" at "); text(panic->file); text(":"); number(panic->line);
        text(":"); number(panic->column);
    }
    if (panic->message_len) {
        text(": ");
        output((const char *)panic->message, panic->message_len <= 192 ? (size_t)panic->message_len : 192);
    }
    text("\n");
}
_Noreturn void __nether_v1_abort_double_panic(const NetherPanic *panic) {
    text("Nether: panic during unwinding\n");
    __nether_v1_report_panic(panic);
    abort();
}

#ifdef NETHER_DIAGNOSTIC_RUNTIME
typedef struct Allocation {
    void *base;
    uint64_t size, alignment, id;
    struct Allocation *next;
} Allocation;
static Allocation *allocations;
static uint64_t next_id = 1, live_count;
uint64_t __nether_v1_debug_live_allocations(void) { return live_count; }
static _Noreturn void invalid_free(void) {
    text("Nether runtime: invalid or duplicate deallocation\n");
    abort();
}
#endif

void *__nether_v1_allocate(uint64_t size, uint64_t alignment) {
    if (!alignment || (alignment & (alignment - 1)) || alignment > INT64_MAX || size > INT64_MAX) return NULL;
    if (!size) return (void *)(uintptr_t)alignment;
    void *base = NULL;
    size_t actual_alignment = alignment < sizeof(void *) ? sizeof(void *) : (size_t)alignment;
    if (posix_memalign(&base, actual_alignment, (size_t)size) != 0) return NULL;
#ifdef NETHER_DIAGNOSTIC_RUNTIME
    Allocation *entry = malloc(sizeof(*entry));
    if (!entry || next_id == UINT64_MAX) { free(entry); free(base); return NULL; }
    *entry = (Allocation){base, size, alignment, next_id++, allocations};
    allocations = entry;
    ++live_count;
    text("allocate #"); number(entry->id); text(" bytes="); number(size); text("\n");
#endif
    return base;
}
void __nether_v1_deallocate(void *base, uint64_t size, uint64_t alignment) {
    if (!size) return;
#ifdef NETHER_DIAGNOSTIC_RUNTIME
    Allocation **entry = &allocations;
    while (*entry && (*entry)->base != base) entry = &(*entry)->next;
    if (!*entry || (*entry)->size != size || (*entry)->alignment != alignment) invalid_free();
    Allocation *old = *entry;
    *entry = old->next;
    --live_count;
    text("free #"); number(old->id); text("\n");
    free(old);
#else
    (void)alignment;
#endif
    free(base);
}

/* Generated code owns these records in its frame. Only initialized owned roots
 * are linked. No heap allocation, tracing or reference counting is involved.
 * Generated drop glue owns per-field initialization flags and field order. */
typedef struct NetherCleanupRoot NetherCleanupRoot;
typedef struct { NetherCleanupRoot *last; } NetherCleanupScope;
typedef bool (*NetherDrop)(NetherPanic *, void *);
struct NetherCleanupRoot {
    NetherCleanupScope *scope;
    NetherCleanupRoot *previous, *next;
    NetherDrop drop;
    void *value;
};
_Static_assert(sizeof(NetherCleanupScope) == 8, "cleanup scope ABI mismatch");
_Static_assert(sizeof(NetherCleanupRoot) == 40, "cleanup root ABI mismatch");

void __nether_v1_cleanup_forget(NetherCleanupRoot *root) {
    if (!root->scope) return;
    if (root->previous) root->previous->next = root->next;
    if (root->next) root->next->previous = root->previous;
    else root->scope->last = root->previous;
    root->scope = NULL;
    root->previous = root->next = NULL;
}

void __nether_v1_cleanup_push(NetherCleanupScope *scope, NetherCleanupRoot *root) {
    /* An occupied destination must be dropped before overwriting its payload.
     * Silently unlinking here would leak the old value. */
    if (root->scope || !root->drop) abort();
    root->scope = scope;
    root->previous = scope->last;
    root->next = NULL;
    if (scope->last) scope->last->next = root;
    scope->last = root;
}

bool __nether_v1_cleanup_root(NetherPanic *panic, NetherCleanupRoot *root) {
    if (!root->scope) return true;
    /* Clear the ownership flag before entering arbitrary destructor code. */
    __nether_v1_cleanup_forget(root);
    bool unwinding = panic->active != 0;
    bool success = root->drop(panic, root->value);
    if (!success) {
        if (unwinding) __nether_v1_abort_double_panic(panic);
        if (!panic->active) abort(); /* malformed generated drop glue */
    }
    return success;
}

bool __nether_v1_cleanup_scope(NetherPanic *panic, NetherCleanupScope *scope) {
    bool success = true;
    while (scope->last) {
        if (!__nether_v1_cleanup_root(panic, scope->last)) success = false;
    }
    return success;
}
