/* The GUI loads these through winit, wayland-sys, glutin and rfd at runtime.
 * --version and ELF DT_NEEDED inspection alone cannot detect their absence.
 * Compile outside the test container so build tools cannot supply missing deps.
 */
#include <dlfcn.h>
#include <stdio.h>

int main(void) {
    const char *libraries[] = {
        "libasound.so.2",
        "libGL.so.1", "libEGL.so.1",
        "libwayland-client.so.0", "libwayland-cursor.so.0", "libwayland-egl.so.1",
        "libxkbcommon.so.0", "libxkbcommon-x11.so.0",
        "libX11.so.6", "libX11-xcb.so.1", "libXcursor.so.1", "libXi.so.6", "libXrandr.so.2",
    };
    int failed = 0;
    for (size_t i = 0; i < sizeof(libraries) / sizeof(libraries[0]); i++) {
        void *handle = dlopen(libraries[i], RTLD_NOW | RTLD_LOCAL);
        if (handle == NULL) {
            fprintf(stderr, "%s: %s\n", libraries[i], dlerror());
            failed = 1;
        } else {
            printf("Loaded %s\n", libraries[i]);
            dlclose(handle);
        }
    }
    return failed;
}
