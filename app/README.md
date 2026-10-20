# Applications

Both applications open the same maps, models and scenario files, so work can move freely between them.

## Web application

[`web/coxeter_mfv.html`](web/coxeter_mfv.html) is the complete application in one file, with the bundled models and examples built in.

- Open it in a recent version of Chrome, Edge, Firefox or Safari, on a computer or a phone.
- It runs entirely in the browser. Nothing is installed and no data is sent anywhere.
- Saved maps, models and scenarios are written as files to your computer.

This file bundles the GLPK solver and is therefore distributed under GPL-3.0.

## Windows desktop application

[`windows/coxeter-mfv.exe`](windows/coxeter-mfv.exe) is a native build for 64-bit Windows 10 and 11.

- Double-click it to start. The bundled models are compiled into the program, so no other files are needed.
- It draws the map on the GPU, solves flux problems with the HiGHS solver, or with Gurobi or CPLEX when they are installed (`coxeter-mfv.exe --solvers` lists them), and spreads flux variability analysis over all processor cores.
- Model and map files given on the command line are opened at start.

```powershell
coxeter-mfv.exe model.json map.json
coxeter-mfv.exe --dark
coxeter-mfv.exe --help
```

Windows may warn the first time that the program comes from an unknown publisher, because the file is not code-signed. Choose *More info* and then *Run anyway* to start it.

To build the desktop application for Linux or macOS, follow [`../code/README.md`](../code/README.md).
