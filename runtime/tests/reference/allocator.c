#include <assert.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/resource.h>
#include <signal.h>
void *__nether_v1_allocate(uint64_t, uint64_t);
void __nether_v1_deallocate(void *, uint64_t, uint64_t);
uint64_t __nether_v1_debug_live_allocations(void);
int main(void) {
    struct rlimit no_core = {0, 0};
    assert(setrlimit(RLIMIT_CORE, &no_core) == 0);
    assert(__nether_v1_allocate(1, 0) == NULL);
    assert(__nether_v1_allocate(1, 3) == NULL);
    assert(__nether_v1_allocate(UINT64_MAX, 8) == NULL);
    for (uint64_t align = 1; align <= 4096; align *= 2) {
        void *empty = __nether_v1_allocate(0, align);
        assert(empty && (uintptr_t)empty % align == 0);
        __nether_v1_deallocate(empty, 0, align);
        for (uint64_t size = 1; size < 1024; size = size * 2 + 1) {
            void *base = __nether_v1_allocate(size, align);
            assert(base && (uintptr_t)base % align == 0);
            memset(base, 0xa5, (size_t)size);
            assert(__nether_v1_debug_live_allocations() == 1);
            __nether_v1_deallocate(base, size, align);
            assert(__nether_v1_debug_live_allocations() == 0);
        }
    }
    void *base = __nether_v1_allocate(16, 8);
    __nether_v1_deallocate(base, 16, 8);
    pid_t child = fork();
    assert(child >= 0);
    if (child == 0) { __nether_v1_deallocate(base, 16, 8); _exit(1); }
    int status;
    assert(waitpid(child, &status, 0) == child);
    assert(WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT);
    assert(__nether_v1_debug_live_allocations() == 0);
    return 0;
}
