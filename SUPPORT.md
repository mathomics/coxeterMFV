# Support

Report bugs and ask usage questions through the [issue tracker](https://github.com/mathomics/coxeterMFV/issues). Include the version, platform, solver, and a minimal example where possible. This is research software; no response time or service-level agreement is promised.

Start with the [user guide](docs/USER_GUIDE.md), [reproducibility guide](docs/REPRODUCIBILITY.md) and [examples](examples/README.md).

## Common problems

| Symptom | What to check |
| --- | --- |
| Downloaded HTML displays source code | Download the actual `coxeter_mfv.html` file, rather than saving GitHub's file preview. Open the downloaded file in a current desktop browser. |
| `code/index.html` is blank when opened directly | The source version needs an HTTP server. Run `npm run serve` from `code/` and open the address printed in the terminal. The ready-to-run file in `app/web/` opens directly. |
| Zuñiga or Yeast9 source download fails | These features request external model files. Check connectivity and try again. The Zuñiga loader verifies the pinned source checksums. |
| No feasible solution | Check the medium, uptake bounds, objective, gene knockouts, maintenance requirements and, for communities, abundances. A map alone does not define a feasible model. |
| Two solvers show different reaction fluxes | Alternative optima can produce different flux vectors. Compare objective values, feasibility and solver tolerances; use pFBA or FVA when appropriate. |
| A commercial solver rejects a problem | Check that its library and licence are available and that any problem-size limit permits this model. HiGHS is included in the desktop application. |
| Windows blocks the executable | The executable is unsigned. Verify its SHA-256 against `release-manifest.json` and obtain it from this repository before deciding whether to run it. |

Avoid posting unpublished, personal or licensed data in a public issue. A stripped-down synthetic example is usually sufficient.
