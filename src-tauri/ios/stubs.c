// iPhone et iPad : remplaçants « faibles » des fonctions du pont Swift
// (gen/apple/Sources/lumen/LumenNative.swift). Cargo lie aussi une bibliothèque
// dynamique où le code Swift de l'app n'est pas : ces définitions la laissent se
// lier. Dans l'app, les vraies fonctions Swift l'emportent toujours.
#include <stddef.h>

typedef void (*lumen_picked)(void *, const char *, const char *);
typedef void (*lumen_closed)(void *);

__attribute__((weak)) void lumen_pick_folder(lumen_picked cb, void *user) { cb(user, NULL, NULL); }
__attribute__((weak)) int lumen_access_begin(const char *mark, char *out, long cap) { (void)mark; (void)out; (void)cap; return 0; }
__attribute__((weak)) void lumen_access_end(const char *path) { (void)path; }
__attribute__((weak)) int lumen_bookmark(const char *path, char *out, long cap) { (void)path; (void)out; (void)cap; return 0; }
__attribute__((weak)) int lumen_materialize(const char *path) { (void)path; return 0; }
__attribute__((weak)) void lumen_haptic(int kind) { (void)kind; }
__attribute__((weak)) int lumen_web_open(const char *url, lumen_closed cb, void *user) { (void)url; (void)cb; (void)user; return 0; }
__attribute__((weak)) void lumen_web_close(void) {}
