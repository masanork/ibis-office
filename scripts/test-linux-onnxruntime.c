/* SPDX-License-Identifier: AGPL-3.0-only */
/* Verify that the packaged native runtime loads on the distribution baseline. */
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <onnxruntime_c_api.h>

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!library) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 1; }
    const OrtApiBase *(ORT_API_CALL *get_base)(void) = dlsym(library, "OrtGetApiBase");
    if (!get_base) { fprintf(stderr, "OrtGetApiBase: %s\n", dlerror()); return 1; }
    const OrtApiBase *base = get_base();
    if (!base || !base->GetApi(ORT_API_VERSION) || strcmp(base->GetVersionString(), "1.22.0") != 0) {
        fprintf(stderr, "Unexpected ONNX Runtime API or version\n");
        return 1;
    }
    puts(base->GetVersionString());
    return dlclose(library) != 0;
}
