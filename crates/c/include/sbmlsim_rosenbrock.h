/* sbmlsim_rosenbrock.h: the Rosenbrock integrator of the QFunction target of sbmlsim-core
   (sbmlsim-c), up to a number of step attempts of RODAS4 per point and application.

   Plain C99 for libCEED: no allocation, no call outside the model and <math.h>, no function
   pointer, every work array of a size the model fixes at compile time. sbmlsim-c pastes this
   file into the QFunction of a model after <ceed/types.h> and <math.h>, the model functions and
   these macros:

     SBMLSIM_ROSENBROCK_N             the number of states, at least 1
     SBMLSIM_ROSENBROCK_N_PARAMETERS  the number of parameters
     SBMLSIM_ROSENBROCK_N_MEMORY      the length of p: the parameters and the constants behind
                                      them, at least 1
     SBMLSIM_ROSENBROCK_CONSTANTS     void (double *p): the constants from the parameters
     SBMLSIM_ROSENBROCK_RHS           void (double t, const double *y, const double *p,
                                      double *out): the right hand side
     SBMLSIM_ROSENBROCK_JAC           the same arguments: the Jacobian in the states, dense,
                                      row major (out[r * N + c] = d rate r / d state c)
     SBMLSIM_ROSENBROCK_DFDT          the same arguments: the derivative of the right hand side
                                      in the time, defined only for a model whose rates depend
                                      on it

   The method is RODAS4 (E. Hairer, G. Wanner, Solving Ordinary Differential Equations II,
   2nd ed., Springer 1996, section VI.4, and their code rodas.f): order 4 with 6 stages and an
   embedded solution of order 3, both stiffly accurate and L-stable, so the error estimate stays
   sound for stiff components; the coefficients and the stage formulation are those of rodas.f.
   One LU decomposition with partial pivoting of the matrix 1/(h gamma) I - J per step.

   The step control is the one of rodas.f without its memory across steps (the points carry
   only t and h): the error is the root mean square of the embedded estimate scaled by
   atol + rtol max(|y|, |y_new|); a step is accepted when it is at most 1 and every new state is
   finite, and the next step is h / q, q = max(1/6, min(5, err^(1/4) / 0.9)). A singular matrix
   halves the step, a state or an error which is not finite divides it by 5. A step which
   reaches t1 within 1.0001 h is stretched or shortened to end exactly at t1; after it the next
   step is the one before the shortening divided by q where q > 1 (h / q where that is larger),
   so a short last step does not shrink the step of the next interval. A point without a step
   (h <= 0 or not finite) starts with the estimate of Hairer and Wanner (their HINIT, one
   explicit Euler step). The integrator reads neither INFINITY nor NAN.

   The control of a point is t, h, done and failed: an attempt is made where done and failed
   are 0; it sets done where t reaches t1, and failed where the step it would take next is
   below h_min (or no longer advances t), keeping the state of the last accepted step. An
   application makes attempts for a point until it is done or failed, at most `attempts` of
   the context. The counts of a point are its accepted and its rejected steps, cumulative. */

#ifndef SBMLSIM_ROSENBROCK_H
#define SBMLSIM_ROSENBROCK_H

/* The context of the QFunction, in the time unit of the model. */
struct SbmlsimRosenbrockContext {
  double t1;    /* the end of the interval */
  double rtol;  /* the relative tolerance */
  double atol;  /* the absolute tolerance */
  double h_min; /* the smallest step: a point which needs a smaller one fails */
  double attempts; /* the most step attempts of a point in one application, at least 1 */
};

/* The root mean square of v scaled by atol + rtol max(|a|, |b|). */
CEED_QFUNCTION_HELPER double sbmlsim_rosenbrock_norm(const double *v, const double *a, const double *b,
                                                     double rtol, double atol) {
  double sum = 0.0;
  for (int k = 0; k < SBMLSIM_ROSENBROCK_N; k++) {
    const double scale = atol + rtol * fmax(fabs(a[k]), fabs(b[k]));
    const double x     = v[k] / scale;
    sum += x * x;
  }
  return sqrt(sum / SBMLSIM_ROSENBROCK_N);
}

/* The LU decomposition of the row major matrix a with partial pivoting, in place: row k was
   swapped with row pivot[k] at the elimination of column k. 0 where a pivot is 0 or not
   finite. */
CEED_QFUNCTION_HELPER int sbmlsim_rosenbrock_lu(double *a, int *pivot) {
  const int n = SBMLSIM_ROSENBROCK_N;
  for (int j = 0; j < n; j++) {
    int    p       = j;
    double largest = fabs(a[j * n + j]);
    for (int r = j + 1; r < n; r++) {
      const double x = fabs(a[r * n + j]);
      if (x > largest) {
        largest = x;
        p       = r;
      }
    }
    pivot[j] = p;
    if (!(largest > 0.0) || !isfinite(largest)) return 0;
    if (p != j) {
      for (int c = 0; c < n; c++) {
        const double x = a[j * n + c];
        a[j * n + c]   = a[p * n + c];
        a[p * n + c]   = x;
      }
    }
    const double d = a[j * n + j];
    for (int r = j + 1; r < n; r++) {
      const double l = a[r * n + j] / d;
      a[r * n + j]   = l;
      for (int c = j + 1; c < n; c++) a[r * n + c] -= l * a[j * n + c];
    }
  }
  return 1;
}

/* Solves a x = b in place of b with the decomposition of sbmlsim_rosenbrock_lu. */
CEED_QFUNCTION_HELPER void sbmlsim_rosenbrock_solve(const double *a, const int *pivot, double *b) {
  const int n = SBMLSIM_ROSENBROCK_N;
  for (int j = 0; j < n; j++) {
    const int p = pivot[j];
    if (p != j) {
      const double x = b[j];
      b[j]           = b[p];
      b[p]           = x;
    }
  }
  for (int r = 1; r < n; r++) {
    double x = b[r];
    for (int c = 0; c < r; c++) x -= a[r * n + c] * b[c];
    b[r] = x;
  }
  for (int r = n - 1; r >= 0; r--) {
    double x = b[r];
    for (int c = r + 1; c < n; c++) x -= a[r * n + c] * b[c];
    b[r] = x / a[r * n + r];
  }
}

/* The first step at (t, y) of the rates f0, at most rest: HINIT of Hairer and Wanner for a
   method of order 4. work is a vector of N. */
CEED_QFUNCTION_HELPER double sbmlsim_rosenbrock_first_step(const struct SbmlsimRosenbrockContext *c, double t,
                                                           double rest, const double *y, const double *p,
                                                           const double *f0, double *y1, double *f1) {
  const double d0 = sbmlsim_rosenbrock_norm(y, y, y, c->rtol, c->atol);
  const double d1 = sbmlsim_rosenbrock_norm(f0, y, y, c->rtol, c->atol);
  double       h0 = (d0 < 1e-5 || d1 < 1e-5) ? 1e-6 : 0.01 * d0 / d1;
  h0              = fmin(h0, rest);
  for (int k = 0; k < SBMLSIM_ROSENBROCK_N; k++) y1[k] = y[k] + h0 * f0[k];
  SBMLSIM_ROSENBROCK_RHS(t + h0, y1, p, f1);
  for (int k = 0; k < SBMLSIM_ROSENBROCK_N; k++) f1[k] -= f0[k];
  const double d2    = sbmlsim_rosenbrock_norm(f1, y, y, c->rtol, c->atol) / h0;
  const double d12   = fmax(d1, d2);
  const double h1    = (d12 <= 1e-15 || !isfinite(d12)) ? fmax(1e-6, h0 * 1e-3) : pow(0.01 / d12, 0.25);
  double       first = fmin(fmin(100.0 * h0, h1), rest);
  return first;
}

/* One step attempt of RODAS4 from (control[0], y) with the step control[1], see the top of
   the file. p holds the parameters and the constants. */
CEED_QFUNCTION_HELPER void sbmlsim_rosenbrock_attempt(const struct SbmlsimRosenbrockContext *c, const double *p,
                                                      double *y, double *control, double *counts) {
  enum { N = SBMLSIM_ROSENBROCK_N };
  /* the coefficients of RODAS4 (rodas.f, METH = 1) */
  const double gamma = 0.25;
  const double c2 = 0.386, c3 = 0.21, c4 = 0.63;
#ifdef SBMLSIM_ROSENBROCK_DFDT
  const double d1 = 0.25, d2 = -0.1043, d3 = 0.1035, d4 = -0.3620000000000023e-01;
#endif
  const double a21 = 0.1544e+01;
  const double a31 = 0.9466785280815826e+00, a32 = 0.2557011698983284e+00;
  const double a41 = 0.3314825187068521e+01, a42 = 0.2896124015972201e+01, a43 = 0.9986419139977817e+00;
  const double a51 = 0.1221224509226641e+01, a52 = 0.6019134481288629e+01, a53 = 0.1253708332932087e+02,
               a54 = -0.6878860361058950e+00;
  const double c21 = -0.5668800000000000e+01;
  const double c31 = -0.2430093356833875e+01, c32 = -0.2063599157091915e+00;
  const double c41 = -0.1073529058151375e+00, c42 = -0.9594562251023355e+01, c43 = -0.2047028614809616e+02;
  const double c51 = 0.7496443313967647e+01, c52 = -0.1024680431464352e+02, c53 = -0.3399990352819905e+02,
               c54 = 0.1170890893206160e+02;
  const double c61 = 0.8083246795921522e+01, c62 = -0.7981132988064893e+01, c63 = -0.3152159432874371e+02,
               c64 = 0.1631930543123136e+02, c65 = -0.6058818238834054e+01;

  const double t    = control[0];
  const double rest = c->t1 - t;
  if (!(rest > 0.0)) {
    control[2] = 1.0;
    return;
  }
  double f0[N], e[N * N], ynew[N], dy[N], k1[N], k2[N], k3[N], k4[N], k5[N], k6[N];
  int    pivot[N];
  SBMLSIM_ROSENBROCK_RHS(t, y, p, f0);
  double h = control[1];
  if (!(h > 0.0) || !isfinite(h)) h = fmax(sbmlsim_rosenbrock_first_step(c, t, rest, y, p, f0, ynew, dy), c->h_min);
  /* the step before it is shortened or stretched to t1 */
  const double proposed = h;
  int          last     = 0;
  if (h * 1.0001 >= rest) {
    h    = rest;
    last = 1;
  }
  if (!last && (h < c->h_min || !(t + h > t))) {
    control[3] = 1.0;
    return;
  }

  /* e = 1/(h gamma) I - J */
  SBMLSIM_ROSENBROCK_JAC(t, y, p, e);
  const double fac = 1.0 / (h * gamma);
  for (int r = 0; r < N; r++) {
    for (int k = 0; k < N; k++) e[r * N + k] = -e[r * N + k];
    e[r * N + r] += fac;
  }
  /* err is valid where the matrix is not singular; finite where every new state is */
  double err      = 0.0;
  int    finite   = 0;
  int    singular = !sbmlsim_rosenbrock_lu(e, pivot);
  if (!singular) {
#ifdef SBMLSIM_ROSENBROCK_DFDT
    double ft[N];
    SBMLSIM_ROSENBROCK_DFDT(t, y, p, ft);
    const double hd1 = h * d1, hd2 = h * d2, hd3 = h * d3, hd4 = h * d4;
#endif
    const double hc21 = c21 / h, hc31 = c31 / h, hc32 = c32 / h, hc41 = c41 / h, hc42 = c42 / h, hc43 = c43 / h;
    const double hc51 = c51 / h, hc52 = c52 / h, hc53 = c53 / h, hc54 = c54 / h;
    const double hc61 = c61 / h, hc62 = c62 / h, hc63 = c63 / h, hc64 = c64 / h, hc65 = c65 / h;
    /* stage 1 */
    for (int k = 0; k < N; k++) {
      k1[k] = f0[k];
#ifdef SBMLSIM_ROSENBROCK_DFDT
      k1[k] += hd1 * ft[k];
#endif
    }
    sbmlsim_rosenbrock_solve(e, pivot, k1);
    /* stage 2 */
    for (int k = 0; k < N; k++) ynew[k] = y[k] + a21 * k1[k];
    SBMLSIM_ROSENBROCK_RHS(t + c2 * h, ynew, p, dy);
    for (int k = 0; k < N; k++) {
      k2[k] = dy[k];
#ifdef SBMLSIM_ROSENBROCK_DFDT
      k2[k] += hd2 * ft[k];
#endif
      k2[k] += hc21 * k1[k];
    }
    sbmlsim_rosenbrock_solve(e, pivot, k2);
    /* stage 3 */
    for (int k = 0; k < N; k++) ynew[k] = y[k] + a31 * k1[k] + a32 * k2[k];
    SBMLSIM_ROSENBROCK_RHS(t + c3 * h, ynew, p, dy);
    for (int k = 0; k < N; k++) {
      k3[k] = dy[k];
#ifdef SBMLSIM_ROSENBROCK_DFDT
      k3[k] += hd3 * ft[k];
#endif
      k3[k] += hc31 * k1[k] + hc32 * k2[k];
    }
    sbmlsim_rosenbrock_solve(e, pivot, k3);
    /* stage 4 */
    for (int k = 0; k < N; k++) ynew[k] = y[k] + a41 * k1[k] + a42 * k2[k] + a43 * k3[k];
    SBMLSIM_ROSENBROCK_RHS(t + c4 * h, ynew, p, dy);
    for (int k = 0; k < N; k++) {
      k4[k] = dy[k];
#ifdef SBMLSIM_ROSENBROCK_DFDT
      k4[k] += hd4 * ft[k];
#endif
      k4[k] += hc41 * k1[k] + hc42 * k2[k] + hc43 * k3[k];
    }
    sbmlsim_rosenbrock_solve(e, pivot, k4);
    /* stage 5, at t + h */
    for (int k = 0; k < N; k++) ynew[k] = y[k] + a51 * k1[k] + a52 * k2[k] + a53 * k3[k] + a54 * k4[k];
    SBMLSIM_ROSENBROCK_RHS(t + h, ynew, p, dy);
    for (int k = 0; k < N; k++) k5[k] = dy[k] + (hc52 * k2[k] + hc54 * k4[k] + hc51 * k1[k] + hc53 * k3[k]);
    sbmlsim_rosenbrock_solve(e, pivot, k5);
    /* the embedded solution, then stage 6, whose solution is the estimate of its error */
    for (int k = 0; k < N; k++) ynew[k] += k5[k];
    SBMLSIM_ROSENBROCK_RHS(t + h, ynew, p, dy);
    for (int k = 0; k < N; k++) k6[k] = dy[k] + (hc61 * k1[k] + hc62 * k2[k] + hc65 * k5[k] + hc64 * k4[k] + hc63 * k3[k]);
    sbmlsim_rosenbrock_solve(e, pivot, k6);
    finite = 1;
    for (int k = 0; k < N; k++) {
      ynew[k] += k6[k];
      finite = finite && isfinite(ynew[k]);
    }
    err    = sbmlsim_rosenbrock_norm(k6, y, ynew, c->rtol, c->atol);
    finite = finite && isfinite(err);
  }

  if (!singular && finite && err <= 1.0) {
    /* accepted */
    const double q    = fmax(1.0 / 6.0, fmin(5.0, pow(err, 0.25) / 0.9));
    const double next = h / q;
    for (int k = 0; k < N; k++) y[k] = ynew[k];
    counts[0] += 1.0;
    if (last) {
      control[0] = c->t1;
      control[2] = 1.0;
      /* the step before it was shortened (or stretched) to t1, scaled by the factor of the
         error where that asks for a smaller one; the step of the error where it is larger. A
         step from the shortened one alone could fall below h_min after a short last step and
         fail the point at the start of the next interval. */
      control[1] = fmax(next, fmin(proposed, proposed / q));
    } else {
      control[0] = t + h;
      control[1] = next;
    }
  } else {
    /* rejected: a singular matrix halves the step, an error which is not finite divides it
       by 5 */
    const double next = singular ? 0.5 * h : (finite ? h / fmin(5.0, pow(err, 0.25) / 0.9) : 0.2 * h);
    counts[1] += 1.0;
    control[1] = next;
    if (next < c->h_min || !(t + next > t)) control[3] = 1.0;
  }
}

/* The body of the QFunction: for each of the Q points (each field [size][Q]), attempts until the
   point is done or failed, at most c->attempts, else a copy. Inputs: state (N), parameters
   (N_PARAMETERS), control (4: t, h, done, failed), counts (2: accepted, rejected); outputs:
   state_out, control_out, counts_out of the same sizes. */
CEED_QFUNCTION_HELPER int sbmlsim_rosenbrock_points(void *ctx, CeedInt Q, const CeedScalar *const *in,
                                                    CeedScalar *const *out) {
  const struct SbmlsimRosenbrockContext *c = (const struct SbmlsimRosenbrockContext *)ctx;
  const CeedScalar *state = in[0], *parameters = in[1], *control = in[2], *counts = in[3];
  CeedScalar       *state_out = out[0], *control_out = out[1], *counts_out = out[2];
  for (CeedInt i = 0; i < Q; i++) {
    double y[SBMLSIM_ROSENBROCK_N], p[SBMLSIM_ROSENBROCK_N_MEMORY], ctl[4], cnt[2];
    for (int k = 0; k < SBMLSIM_ROSENBROCK_N; k++) y[k] = state[k * Q + i];
    for (int k = 0; k < SBMLSIM_ROSENBROCK_N_PARAMETERS; k++) p[k] = parameters[k * Q + i];
    for (int k = 0; k < 4; k++) ctl[k] = control[k * Q + i];
    for (int k = 0; k < 2; k++) cnt[k] = counts[k * Q + i];
    if (ctl[2] == 0.0 && ctl[3] == 0.0) {
      SBMLSIM_ROSENBROCK_CONSTANTS(p);
      for (double a = 0.0; a < c->attempts && ctl[2] == 0.0 && ctl[3] == 0.0; a += 1.0)
        sbmlsim_rosenbrock_attempt(c, p, y, ctl, cnt);
    }
    for (int k = 0; k < SBMLSIM_ROSENBROCK_N; k++) state_out[k * Q + i] = y[k];
    for (int k = 0; k < 4; k++) control_out[k * Q + i] = ctl[k];
    for (int k = 0; k < 2; k++) counts_out[k * Q + i] = cnt[k];
  }
  return 0;
}

#endif
