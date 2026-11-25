/* A CSC SUNMatrix whose immutable pattern survives zeroing and Jacobian copies.
 * Its sparse prefix remains compatible with SUNDIALS and KLU. */
#include <stdlib.h>
#include <string.h>

#include <sunmatrix/sunmatrix_sparse.h>

typedef struct
{
  size_t references;
  sunindextype n, source_nnz, nnz, differential;
  int ode;
  sunindextype *columns, *rows, *mapping, *diagonal;
  unsigned char* present;
} FixedPattern;

typedef struct
{
  struct _SUNMatrixContent_Sparse sparse;
  FixedPattern* pattern;
} FixedContent;

SUNMatrix sbmlsim_SUNSparseMatrix_Fixed(sunindextype n, sunindextype capacity,
                                       SUNContext context);
SUNErrCode sbmlsim_SUNSparseMatrix_SetJacobian(
  SUNMatrix A, sunindextype source_nnz, const sunindextype* columns,
  const sunindextype* rows, const sunrealtype* values, sunindextype differential,
  int ode, sunrealtype cj);

static void release_pattern(FixedPattern* p)
{
  if (p == NULL || --p->references != 0) { return; }
  free(p->columns);
  free(p->rows);
  free(p->mapping);
  free(p->diagonal);
  free(p->present);
  free(p);
}

static void install_pattern(SUNMatrix A, FixedPattern* p)
{
  FixedContent* c = (FixedContent*)A->content;
  if (c->pattern == p) { return; }
  p->references++;
  release_pattern(c->pattern);
  c->pattern = p;
  memcpy(SM_INDEXPTRS_S(A), p->columns,
         ((size_t)p->n + 1) * sizeof(sunindextype));
  memcpy(SM_INDEXVALS_S(A), p->rows, (size_t)p->nnz * sizeof(sunindextype));
}

static SUNMatrix clone_fixed(SUNMatrix A)
{
  SUNMatrix B = sbmlsim_SUNSparseMatrix_Fixed(SM_COLUMNS_S(A), SM_NNZ_S(A), A->sunctx);
  FixedPattern* p = ((FixedContent*)A->content)->pattern;
  if (B != NULL && p != NULL) { install_pattern(B, p); }
  return B;
}

static void destroy_fixed(SUNMatrix A)
{
  if (A != NULL && A->content != NULL)
  {
    release_pattern(((FixedContent*)A->content)->pattern);
  }
  SUNMatDestroy_Sparse(A);
}

static SUNErrCode zero_fixed(SUNMatrix A)
{
  memset(SM_DATA_S(A), 0, (size_t)SM_NNZ_S(A) * sizeof(sunrealtype));
  return SUN_SUCCESS;
}

static SUNErrCode copy_fixed(SUNMatrix A, SUNMatrix B)
{
  FixedPattern* p = ((FixedContent*)A->content)->pattern;
  if (B->ops->clone != clone_fixed || SM_COLUMNS_S(A) != SM_COLUMNS_S(B))
  {
    return SUN_ERR_ARG_INCOMPATIBLE;
  }
  if (p == NULL) { return zero_fixed(B); }
  if (SM_NNZ_S(B) < p->nnz) { return SUN_ERR_ARG_DIMSMISMATCH; }
  install_pattern(B, p);
  memcpy(SM_DATA_S(B), SM_DATA_S(A), (size_t)p->nnz * sizeof(sunrealtype));
  return SUN_SUCCESS;
}

static SUNErrCode scale_add_identity_fixed(sunrealtype scale, SUNMatrix A)
{
  FixedPattern* p = ((FixedContent*)A->content)->pattern;
  sunrealtype* data = SM_DATA_S(A);
  if (p == NULL || p->differential != p->n) { return SUN_ERR_ARG_INCOMPATIBLE; }
  for (sunindextype k = 0; k < p->nnz; k++) { data[k] *= scale; }
  for (sunindextype j = 0; j < p->n; j++)
  {
    data[p->diagonal[j]] = SUN_RCONST(1.0) + data[p->diagonal[j]];
  }
  return SUN_SUCCESS;
}

static SUNErrCode scale_add_fixed(sunrealtype scale, SUNMatrix A, SUNMatrix B)
{
  FixedPattern* p = ((FixedContent*)A->content)->pattern;
  if (B->ops->clone != clone_fixed || p == NULL ||
      p != ((FixedContent*)B->content)->pattern)
  {
    return SUN_ERR_ARG_INCOMPATIBLE;
  }
  for (sunindextype k = 0; k < p->nnz; k++)
  {
    SM_DATA_S(A)[k] = scale * SM_DATA_S(A)[k] + SM_DATA_S(B)[k];
  }
  return SUN_SUCCESS;
}

SUNMatrix sbmlsim_SUNSparseMatrix_Fixed(sunindextype n, sunindextype capacity,
                                       SUNContext context)
{
  SUNMatrix A = SUNSparseMatrix(n, n, capacity, CSC_MAT, context);
  FixedContent* c;
  if (A == NULL) { return NULL; }
  c = (FixedContent*)calloc(1, sizeof(FixedContent));
  if (c == NULL)
  {
    SUNMatDestroy_Sparse(A);
    return NULL;
  }
  c->sparse = *(SUNMatrixContent_Sparse)A->content;
  /* The sparse structure contains pointers to its own index pointer fields. */
  c->sparse.rowvals = &c->sparse.indexvals;
  c->sparse.colptrs = &c->sparse.indexptrs;
  free(A->content);
  A->content = c;
  A->ops->clone = clone_fixed;
  A->ops->destroy = destroy_fixed;
  A->ops->zero = zero_fixed;
  A->ops->copy = copy_fixed;
  A->ops->scaleaddi = scale_add_identity_fixed;
  A->ops->scaleadd = scale_add_fixed;
  return A;
}

SUNErrCode sbmlsim_SUNSparseMatrix_SetJacobian(
  SUNMatrix A, sunindextype source_nnz, const sunindextype* columns,
  const sunindextype* rows, const sunrealtype* values, sunindextype differential,
  int ode, sunrealtype cj)
{
  FixedContent* c = (FixedContent*)A->content;
  FixedPattern* p = c->pattern;
  const sunindextype n = SM_COLUMNS_S(A);
  if (source_nnz < 0 || differential < 0 || differential > n ||
      columns[0] != 0 || columns[n] != source_nnz)
  {
    return SUN_ERR_ARG_INCOMPATIBLE;
  }
  if (p == NULL)
  {
    sunindextype out = 0;
    p = (FixedPattern*)calloc(1, sizeof(FixedPattern));
    if (p == NULL) { return SUN_ERR_MALLOC_FAIL; }
    p->references = 1;
    p->n = n;
    p->source_nnz = source_nnz;
    p->differential = differential;
    p->ode = ode;
    p->columns = (sunindextype*)malloc(((size_t)n + 1) * sizeof(sunindextype));
    p->rows = (sunindextype*)malloc((size_t)SM_NNZ_S(A) * sizeof(sunindextype));
    p->mapping = (sunindextype*)malloc(((size_t)source_nnz + 1) * sizeof(sunindextype));
    p->diagonal = (sunindextype*)malloc(((size_t)differential + 1) * sizeof(sunindextype));
    p->present = (unsigned char*)calloc((size_t)differential + 1, 1);
    if (!p->columns || !p->rows || !p->mapping || !p->diagonal || !p->present)
    {
      release_pattern(p);
      return SUN_ERR_MALLOC_FAIL;
    }
    for (sunindextype j = 0; j < n; j++)
    {
      const sunindextype begin = columns[j], end = columns[j + 1];
      int missing = j < differential;
      if (begin < 0 || begin > end || end > source_nnz)
      {
        release_pattern(p);
        return SUN_ERR_ARG_INCOMPATIBLE;
      }
      for (sunindextype k = begin; k < end; k++)
      {
        if (rows[k] == j) { missing = 0; }
      }
      p->columns[j] = out;
      /* CVODE's sparse ScaleAddI prepends a missing diagonal. IDA inserts it
       * in row order. Retain each ordering for KLU's numeric pivot decisions. */
      if (missing && ode)
      {
        if (out >= SM_NNZ_S(A)) { release_pattern(p); return SUN_ERR_ARG_DIMSMISMATCH; }
        p->diagonal[j] = out;
        p->rows[out++] = j;
        missing = 0;
      }
      for (sunindextype k = begin; k < end; k++)
      {
        if (rows[k] < 0 || rows[k] >= n)
        {
          release_pattern(p);
          return SUN_ERR_ARG_INCOMPATIBLE;
        }
        if (missing && rows[k] > j)
        {
          if (out >= SM_NNZ_S(A)) { release_pattern(p); return SUN_ERR_ARG_DIMSMISMATCH; }
          p->diagonal[j] = out;
          p->rows[out++] = j;
          missing = 0;
        }
        if (out >= SM_NNZ_S(A)) { release_pattern(p); return SUN_ERR_ARG_DIMSMISMATCH; }
        p->mapping[k] = out;
        p->rows[out] = rows[k];
        if (rows[k] == j && j < differential)
        {
          p->diagonal[j] = out;
          p->present[j] = 1;
        }
        out++;
      }
      if (missing)
      {
        if (out >= SM_NNZ_S(A)) { release_pattern(p); return SUN_ERR_ARG_DIMSMISMATCH; }
        p->diagonal[j] = out;
        p->rows[out++] = j;
      }
    }
    p->columns[n] = out;
    p->nnz = out;
    install_pattern(A, p);
    release_pattern(p);
    p = c->pattern;
  }
  else if (p->source_nnz != source_nnz || p->differential != differential || p->ode != ode)
  {
    return SUN_ERR_ARG_INCOMPATIBLE;
  }
  /* SUNDIALS zeroed numeric values before the callback. Only absent diagonal
   * entries need explicit values, since every model entry is overwritten. */
  if (p->nnz == source_nnz)
  {
    memcpy(SM_DATA_S(A), values, (size_t)source_nnz * sizeof(sunrealtype));
  }
  else
  {
    for (sunindextype k = 0; k < source_nnz; k++)
    {
      SM_DATA_S(A)[p->mapping[k]] = values[k];
    }
  }
  for (sunindextype j = 0; j < differential; j++)
  {
    sunrealtype* value = &SM_DATA_S(A)[p->diagonal[j]];
    if (!p->present[j]) { *value = ode ? SUN_RCONST(0.0) : -cj; }
    else if (!ode) { *value -= cj; }
  }
  return SUN_SUCCESS;
}
