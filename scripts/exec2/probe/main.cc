#include <cstdio>
int util_value();
int b_value();
extern "C" int c_value(void);
int main() {
  std::printf("%d %d %d\n", util_value(), b_value(), c_value());
  return 0;
}
