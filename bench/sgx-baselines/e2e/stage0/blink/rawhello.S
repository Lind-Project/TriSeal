  .globl _start
_start:
  mov $1, %eax
  mov $1, %edi
  lea msg(%rip), %rsi
  mov $len, %edx
  syscall
  mov $60, %eax
  xor %edi, %edi
  syscall
msg: .ascii "hello from x86_64 under lind blink\n"
  .set len, . - msg
