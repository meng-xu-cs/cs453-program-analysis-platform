#include "interface.h"

int main(void) {
    char input[10] = {0};
    int len = in(input, 10);
    if (len > 0) {
        if (input[0] == 'a') {
            abort();
        }
    }
    out(input);
    return 0;
}
