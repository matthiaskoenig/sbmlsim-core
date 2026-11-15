/*
 * The KLU linear solver of SUNDIALS (`SUNLinSol_KLU`), which keeps its symbolic analysis across
 * the initializations of the solver.
 *
 * CVODE initializes its linear solver at every `CVodeReInit` (`cvInitialSetup` ->
 * `cvLsInitialize` -> `SUNLinSolInitialize_KLU`), which sets `first_factorize`, so the next
 * `SUNLinSolSetup_KLU` analyzes the pattern again (`klu_analyze`: BTF, AMD or COLAMD) before it
 * factorizes. The pattern of the Jacobian of a model does not change between restarts, so the
 * analysis comes out the same every time. This solver replaces the setup of `SUNLinSol_KLU`: on
 * the first factorization after an initialization, when a symbolic analysis exists and the
 * pattern of the matrix (the column pointers and the row indices) and the settings of KLU the
 * analysis reads are the ones it was made for, it frees the numeric factorization and factorizes
 * afresh (`klu_factor`, with partial pivoting) with the kept analysis. That is exactly what
 * `SUNLinSolSetup_KLU` does after its analysis, which is a function of the pattern and those
 * settings only, so the results are bitwise the same. In every other case it calls
 * `SUNLinSolSetup_KLU` and records the pattern of a new analysis.
 */

#include <stdlib.h>
#include <string.h>

#include <sundials/sundials_linearsolver.h>
#include <sunlinsol/sunlinsol_klu.h>
#include <sunmatrix/sunmatrix_sparse.h>

/* The content of the solver. The operations of `SUNLinSol_KLU` cast the content to its own type,
 * so that comes first. */
typedef struct
{
  struct _SUNLinearSolverContent_KLU klu;
  /* whether `klu.symbolic` is the analysis of the pattern and the settings below */
  int recorded;
  /* the columns of the pattern and the `colptrs[n]` entries of the pattern */
  sunindextype n;
  sunindextype nnz;
  /* the `n + 1` column pointers and the `nnz` row indices, with their capacities */
  sunindextype* colptrs;
  sunindextype* rowvals;
  sunindextype colptrs_capacity;
  sunindextype rowvals_capacity;
  /* the settings of KLU at the analysis: `klu_analyze` reads `ordering`, `btf`, `maxwork`,
   * `user_order` and `user_data` */
  sun_klu_common settings;
  /* the symbolic analyses made and the factorizations which reused one */
  long int analyses;
  long int reuses;
} KeptContent;

SUNLinearSolver sbmlsim_SUNLinSol_KLUKept(N_Vector y, SUNMatrix A, SUNContext sunctx);
SUNErrCode sbmlsim_SUNLinSol_KLUKeptStats(SUNLinearSolver S, long int* analyses,
                                          long int* reuses);

/* Whether the symbolic analysis of `c` was made for the pattern of `A` and the current settings. */
static int same_analysis(const KeptContent* c, SUNMatrix A)
{
  const sun_klu_common* now = &c->klu.common;
  const sun_klu_common* then = &c->settings;
  sunindextype n               = SUNSparseMatrix_NP(A);
  const sunindextype* colptrs  = SUNSparseMatrix_IndexPointers(A);
  const sunindextype* rowvals  = SUNSparseMatrix_IndexValues(A);

  if (!c->recorded || c->klu.symbolic == NULL || n != c->n || colptrs[n] != c->nnz)
  {
    return 0;
  }
  if (now->ordering != then->ordering || now->btf != then->btf ||
      now->maxwork != then->maxwork || now->user_order != then->user_order ||
      now->user_data != then->user_data)
  {
    return 0;
  }
  return memcmp(colptrs, c->colptrs, (size_t)(n + 1) * sizeof(sunindextype)) == 0 &&
         memcmp(rowvals, c->rowvals, (size_t)c->nnz * sizeof(sunindextype)) == 0;
}

/* Grows `*buffer` to at least `size` indices; returns 0 when the memory is not there. */
static int reserve(sunindextype** buffer, sunindextype* capacity, sunindextype size)
{
  sunindextype* grown;
  if (size <= *capacity) { return 1; }
  grown = (sunindextype*)realloc(*buffer, (size_t)size * sizeof(sunindextype));
  if (grown == NULL) { return 0; }
  *buffer   = grown;
  *capacity = size;
  return 1;
}

/* Records the pattern of `A` and the current settings as the ones of the symbolic analysis. */
static void record(KeptContent* c, SUNMatrix A)
{
  sunindextype n              = SUNSparseMatrix_NP(A);
  const sunindextype* colptrs = SUNSparseMatrix_IndexPointers(A);
  const sunindextype* rowvals = SUNSparseMatrix_IndexValues(A);
  sunindextype nnz            = colptrs[n];

  c->recorded = 0;
  if (!reserve(&c->colptrs, &c->colptrs_capacity, n + 1) ||
      !reserve(&c->rowvals, &c->rowvals_capacity, nnz > 0 ? nnz : 1))
  {
    return;
  }
  memcpy(c->colptrs, colptrs, (size_t)(n + 1) * sizeof(sunindextype));
  memcpy(c->rowvals, rowvals, (size_t)nnz * sizeof(sunindextype));
  c->n        = n;
  c->nnz      = nnz;
  c->settings = c->klu.common;
  c->recorded = 1;
}

static int setup_kept(SUNLinearSolver S, SUNMatrix A)
{
  KeptContent* c = (KeptContent*)S->content;
  int analyze    = 0;
  int flag;

  if (SUNMatGetID(A) == SUNMATRIX_SPARSE && c->klu.first_factorize)
  {
    if (same_analysis(c, A))
    {
      /* what `SUNLinSolSetup_KLU` does after `klu_analyze` */
      if (c->klu.numeric) { sun_klu_free_numeric(&c->klu.numeric, &c->klu.common); }
      c->klu.numeric = sun_klu_factor(SUNSparseMatrix_IndexPointers(A),
                                      SUNSparseMatrix_IndexValues(A),
                                      SUNSparseMatrix_Data(A), c->klu.symbolic,
                                      &c->klu.common);
      if (c->klu.numeric == NULL)
      {
        c->klu.last_flag = SUN_ERR_EXT_FAIL;
        return c->klu.last_flag;
      }
      c->klu.first_factorize = 0;
      c->reuses += 1;
      c->klu.last_flag = SUN_SUCCESS;
      return c->klu.last_flag;
    }
    analyze = 1;
  }

  flag = SUNLinSolSetup_KLU(S, A);
  if (analyze)
  {
    if (c->klu.symbolic != NULL)
    {
      c->analyses += 1;
      record(c, A);
    }
    else { c->recorded = 0; }
  }
  return flag;
}

static SUNErrCode free_kept(SUNLinearSolver S)
{
  if (S == NULL) { return SUN_SUCCESS; }
  if (S->content)
  {
    KeptContent* c = (KeptContent*)S->content;
    free(c->colptrs);
    free(c->rowvals);
    c->colptrs = NULL;
    c->rowvals = NULL;
  }
  /* frees the factorizations, the content (one allocation with `klu`), the operations and `S` */
  return SUNLinSolFree_KLU(S);
}

/* `SUNLinSol_KLU(y, A, sunctx)` with the setup which keeps the symbolic analysis. */
SUNLinearSolver sbmlsim_SUNLinSol_KLUKept(N_Vector y, SUNMatrix A, SUNContext sunctx)
{
  SUNLinearSolver S = SUNLinSol_KLU(y, A, sunctx);
  KeptContent* c;

  if (S == NULL) { return NULL; }
  c = (KeptContent*)calloc(1, sizeof *c);
  if (c == NULL)
  {
    SUNLinSolFree(S);
    return NULL;
  }
  /* the content of `SUNLinSol_KLU` holds no pointer into itself */
  c->klu = *(SUNLinearSolverContent_KLU)S->content;
  free(S->content);
  S->content    = c;
  S->ops->setup = setup_kept;
  S->ops->free  = free_kept;
  return S;
}

/* The symbolic analyses `S` made and the factorizations which reused a kept one. */
SUNErrCode sbmlsim_SUNLinSol_KLUKeptStats(SUNLinearSolver S, long int* analyses,
                                          long int* reuses)
{
  const KeptContent* c;
  if (S == NULL || S->content == NULL || S->ops->setup != setup_kept)
  {
    return SUN_ERR_ARG_CORRUPT;
  }
  c = (const KeptContent*)S->content;
  if (analyses) { *analyses = c->analyses; }
  if (reuses) { *reuses = c->reuses; }
  return SUN_SUCCESS;
}
