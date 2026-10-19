#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <signal.h>
#include <sys/wait.h>
#include <unistd.h>

typedef struct {
    uint32_t active, kind;
    const char *file;
    uint32_t line, column;
    uint64_t message_len;
    unsigned char message[192];
} Panic;
typedef bool (*Drop)(Panic *, void *);
typedef struct Root Root;
typedef struct { Root *last; } Scope;
struct Root { Scope *scope; Root *previous, *next; Drop drop; void *value; };
typedef struct { void *value; Drop drop; Root *root; Scope *owner; uint8_t initialized; } Slot;
void *__nether_v1_allocate(uint64_t, uint64_t);
void __nether_v1_deallocate(void *, uint64_t, uint64_t);
uint64_t __nether_v1_debug_live_allocations(void);
void __nether_v1_slot_initialize(Slot *);
void __nether_v1_slot_forget(Slot *);
bool __nether_v1_slot_drop(Panic *, void *);
bool __nether_v1_cleanup_scope(Panic *, Scope *);

static unsigned log_ids[16], length;
static unsigned *make(unsigned id) {
    unsigned *p = __nether_v1_allocate(sizeof(unsigned), _Alignof(unsigned));
    assert(p);
    *p = id;
    return p;
}
static bool drop(Panic *panic, void *value) {
    (void)panic;
    unsigned **p = value;
    assert(length < 16);
    log_ids[length++] = **p;
    __nether_v1_deallocate(*p, sizeof(unsigned), _Alignof(unsigned));
    *p = NULL;
    return true;
}
static void expected(const unsigned *ids, unsigned count) {
    assert(count == length);
    for (unsigned i = 0; i < count; ++i) assert(ids[i] == log_ids[i]);
    length = 0;
}
static void replace(Panic *panic, Slot *view, unsigned id) {
    unsigned *fresh = make(id);
    assert(__nether_v1_slot_drop(panic, view));
    *(unsigned **)view->value = fresh;
    __nether_v1_slot_initialize(view);
}
static Slot *finalizing;
static unsigned forbidden_operation;
static bool recursive_drop(Panic *panic, void *value) {
    (void)value;
    if (forbidden_operation == 0) __nether_v1_slot_initialize(finalizing);
    if (forbidden_operation == 1) __nether_v1_slot_forget(finalizing);
    if (forbidden_operation == 2) (void)__nether_v1_slot_drop(panic, finalizing);
    return true;
}
static void rejects_finalizing_mutation(void) {
    for (forbidden_operation = 0; forbidden_operation < 3; ++forbidden_operation) {
        pid_t child = fork();
        assert(child >= 0);
        if (child == 0) {
            Panic panic = {0};
            unsigned value = 0;
            Slot slot = {&value, recursive_drop, NULL, NULL, 0};
            finalizing = &slot;
            __nether_v1_slot_initialize(&slot);
            (void)__nether_v1_slot_drop(&panic, &slot);
            _exit(1);
        }
        int status;
        assert(waitpid(child, &status, 0) == child);
        assert(WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT);
    }
}
int main(void) {
    rejects_finalizing_mutation();
    Panic panic = {0};
    Scope outer = {0}, inner = {0};
    Root ra = {0}, rb = {0}, rc = {0};
    unsigned *a = make(1), *b = make(2), *c = NULL;
    Slot sa = {&a, drop, &ra, &outer, 0};
    Slot sb = {&b, drop, &rb, &outer, 0};
    Slot sc = {&c, drop, &rc, &inner, 0};
    __nether_v1_slot_initialize(&sa);
    __nether_v1_slot_initialize(&sb);
    replace(&panic, &sa, 3);
    assert(__nether_v1_cleanup_scope(&panic, &outer));
    expected((unsigned[]){1, 3, 2}, 3);
    assert(__nether_v1_debug_live_allocations() == 0);

    a = make(4);
    __nether_v1_slot_initialize(&sa);
    c = a;
    __nether_v1_slot_forget(&sa);
    __nether_v1_slot_initialize(&sc);
    assert(__nether_v1_cleanup_scope(&panic, &outer));
    expected(NULL, 0);
    assert(__nether_v1_debug_live_allocations() == 1);
    assert(__nether_v1_cleanup_scope(&panic, &inner));
    expected((unsigned[]){4}, 1);
    assert(__nether_v1_debug_live_allocations() == 0);

    /* A partially constructed aggregate cleans initialized fields only. The
     * uninitialized second field contains deliberately invalid storage bits. */
    a = make(5);
    Slot first = {&a, drop, NULL, NULL, 0};
    Slot second = {(void *)(uintptr_t)1, drop, NULL, NULL, 0};
    __nether_v1_slot_initialize(&first);
    panic.active = 1;
    assert(__nether_v1_slot_drop(&panic, &first));
    assert(__nether_v1_slot_drop(&panic, &second));
    assert(__nether_v1_slot_drop(&panic, &first));
    expected((unsigned[]){5}, 1);
    assert(__nether_v1_debug_live_allocations() == 0);

    panic.active = 0;
    a = make(6);
    __nether_v1_slot_initialize(&sa);
    for (unsigned i = 0; i < 10000; ++i) {
        replace(&panic, &sa, i);
        assert(__nether_v1_debug_live_allocations() == 1);
        length = 0;
    }
    assert(__nether_v1_cleanup_scope(&panic, &outer));
    assert(__nether_v1_debug_live_allocations() == 0);
    return 0;
}
