/* The sizes of the objects of SUNDIALS around their data, for the estimates of the memory of
 * an integrator: what an N_Vector of the serial implementation holds besides its values. */
#include <stddef.h>

#include <nvector/nvector_serial.h>

size_t sbmlsim_nvector_serial_object_bytes(void);

/* The generic vector, its table of operations (each vector has its own copy, also a clone)
 * and the content of the serial implementation. */
size_t sbmlsim_nvector_serial_object_bytes(void)
{
  return sizeof(struct _generic_N_Vector) + sizeof(struct _generic_N_Vector_Ops) +
         sizeof(struct _N_VectorContent_Serial);
}
