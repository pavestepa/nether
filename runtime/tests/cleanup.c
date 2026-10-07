#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <signal.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

typedef struct {
    uint32_t active, kind;
    const char *file;
    uint32_t line, column;
    uint64_t message_len;
    unsigned char message[192];
} Panic;
typedef struct Root Root;
typedef struct { Root *last; } Scope;
struct Root {
    Scope *scope;
    Root *previous, *next;
    bool (*drop)(Panic *, void *);
    void *value;
};
void __nether_v1_cleanup_forget(Root *);
void __nether_v1_cleanup_push(Scope *, Root *);
bool __nether_v1_cleanup_root(Panic *, Root *);
bool __nether_v1_cleanup_scope(Panic *, Scope *);

static unsigned log_values[32], log_length;
typedef struct { unsigned id; bool fail; Root *root; } Resource;
static bool drop(Panic *panic, void *value) {
    Resource *resource = value;
    assert(!resource->root->scope); /* cleared before callback */
    assert(log_length < 32);
    log_values[log_length++] = resource->id;
    if (resource->fail) { panic->active = 1; panic->kind = 1; return false; }
    return true;
}
static Root root(Resource *value) {
    return (Root){NULL, NULL, NULL, drop, value};
}
static void expected(const unsigned *values, unsigned length) {
    assert(log_length == length);
    for (unsigned i = 0; i < length; ++i) assert(log_values[i] == values[i]);
    log_length = 0;
}
static void generated_sequences(void) {
    enum { COUNT = 8 };
    Resource resources[COUNT];
    Root roots[COUNT];
    Scope scopes[2] = {{0}, {0}};
    unsigned order[2][COUNT], lengths[2] = {0, 0};
    int owners[COUNT];
    for (unsigned i = 0; i < COUNT; ++i) {
        resources[i] = (Resource){i, false, &roots[i]};
        roots[i] = root(&resources[i]);
        owners[i] = -1;
    }
    Panic panic = {0};
    uint32_t random = 73519;
    for (unsigned step = 0; step < 100000; ++step) {
        random = random * 1664525u + 1013904223u;
        unsigned id = (random >> 16) % COUNT, scope = (random >> 20) & 1;
        unsigned operation = (random >> 24) % 4;
        if (operation == 0 && owners[id] < 0) {
            __nether_v1_cleanup_push(&scopes[scope], &roots[id]);
            owners[id] = (int)scope;
            order[scope][lengths[scope]++] = id;
        } else if (operation == 1 || operation == 2) {
            int owner = owners[id];
            if (operation == 1) __nether_v1_cleanup_forget(&roots[id]);
            else assert(__nether_v1_cleanup_root(&panic, &roots[id]));
            if (owner >= 0) {
                unsigned at = 0;
                while (order[owner][at] != id) ++at;
                for (unsigned i = at + 1; i < lengths[owner]; ++i)
                    order[owner][i - 1] = order[owner][i];
                --lengths[owner];
                owners[id] = -1;
            }
            expected(&id, operation == 2 && owner >= 0 ? 1 : 0);
        } else if (operation == 3) {
            unsigned reversed[COUNT], length = lengths[scope];
            for (unsigned i = 0; i < length; ++i) {
                unsigned popped = order[scope][length - 1 - i];
                reversed[i] = popped;
                owners[popped] = -1;
            }
            assert(__nether_v1_cleanup_scope(&panic, &scopes[scope]));
            expected(reversed, length);
            lengths[scope] = 0;
        }
        for (unsigned s = 0; s < 2; ++s) {
            Root *current = scopes[s].last, *next = NULL;
            for (unsigned i = lengths[s]; i > 0; --i) {
                assert(current == &roots[order[s][i - 1]]);
                assert(current->scope == &scopes[s] && current->next == next);
                next = current;
                current = current->previous;
            }
            assert(!current);
        }
    }
    for (unsigned s = 0; s < 2; ++s) {
        assert(__nether_v1_cleanup_scope(&panic, &scopes[s]));
        log_length = 0;
    }
}
int main(void) {
    struct rlimit no_core = {0, 0};
    assert(setrlimit(RLIMIT_CORE, &no_core) == 0);
    Panic panic = {0};
    Scope scope = {0}, nested = {0};
    Resource a = {1, false, NULL}, b = {2, false, NULL}, c = {4, false, NULL};
    Root ra = root(&a), rb = root(&b), rc = root(&c);
    a.root = &ra; b.root = &rb; c.root = &rc;
    __nether_v1_cleanup_push(&scope, &ra);
    __nether_v1_cleanup_push(&scope, &rb);
    assert(__nether_v1_cleanup_root(&panic, &ra));
    a.id = 3;
    __nether_v1_cleanup_push(&scope, &ra);
    assert(__nether_v1_cleanup_scope(&panic, &scope));
    expected((unsigned[]){1, 3, 2}, 3);
    assert(!scope.last && !ra.scope && !rb.scope);
    assert(__nether_v1_cleanup_scope(&panic, &scope));
    expected(NULL, 0);

    /* Moving a root out, then initializing it from an inner scope must keep
     * its original owning scope, with its new initialization position. */
    __nether_v1_cleanup_push(&scope, &ra);
    __nether_v1_cleanup_push(&scope, &rb);
    __nether_v1_cleanup_forget(&ra);
    __nether_v1_cleanup_push(&nested, &rc);
    a.id = 5;
    __nether_v1_cleanup_push(&scope, &ra);
    assert(__nether_v1_cleanup_scope(&panic, &nested));
    assert(scope.last == &ra);
    assert(__nether_v1_cleanup_scope(&panic, &scope));
    expected((unsigned[]){4, 5, 2}, 3);

    /* A first destructor panic still drains the remaining scope. */
    a.fail = true;
    __nether_v1_cleanup_push(&scope, &rb);
    __nether_v1_cleanup_push(&scope, &ra);
    assert(!__nether_v1_cleanup_scope(&panic, &scope));
    assert(panic.active && !scope.last);
    expected((unsigned[]){5, 2}, 2);

    /* A successful destructor while unwinding does not create another panic. */
    __nether_v1_cleanup_push(&scope, &rb);
    assert(__nether_v1_cleanup_scope(&panic, &scope));
    assert(panic.active);
    expected((unsigned[]){2}, 1);

    pid_t child = fork();
    assert(child >= 0);
    if (child == 0) {
        __nether_v1_cleanup_push(&scope, &ra);
        __nether_v1_cleanup_scope(&panic, &scope);
        _exit(1);
    }
    int status;
    assert(waitpid(child, &status, 0) == child);
    assert(WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT);

    /* Long replacement workloads reuse a fixed set of stack records. */
    panic.active = 0; a.fail = false;
    __nether_v1_cleanup_push(&scope, &rb);
    for (unsigned i = 0; i < 100000; ++i) {
        __nether_v1_cleanup_push(&scope, &ra);
        assert(scope.last == &ra && ra.previous == &rb && !rb.previous);
        assert(__nether_v1_cleanup_root(&panic, &ra));
        assert(scope.last == &rb && !rb.next);
        log_length = 0;
    }
    assert(__nether_v1_cleanup_scope(&panic, &scope));
    expected((unsigned[]){2}, 1);
    generated_sequences();
    return 0;
}
