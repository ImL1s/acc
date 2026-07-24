/* Regression: GCC labels-as-values for computed goto dispatch tables.
 * postgres ExecInterpExpr uses `static const void *const dispatch_table[] = {
 *   &&CASE_op, ... };` — parsing `&&label` as 0 made initdb SEGV in PortalRun. */
#include <stddef.h>

int main(void) {
  static const void *const table[] = { &&L_done };
  goto *table[0];
  return 1;
  L_done:
  return 0;
}
