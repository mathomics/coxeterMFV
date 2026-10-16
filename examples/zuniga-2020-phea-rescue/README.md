# *Synechococcus elongatus* and *Escherichia coli* ΔpheA

<img src="figure/coxeter-zuniga-figura.png" alt="Phenylalanine rescue of E. coli ΔpheA by S. elongatus" width="100%">

A phototroph feeds a heterotroph. An *E. coli* mutant that cannot make phenylalanine grows in coculture with a sucrose-secreting strain of *S. elongatus* that supplies it.

Coxeter-MFV solves the authors' original models and reproduces their predictions within an absolute tolerance of 10⁻⁸. The ΔpheA monoculture does not grow, while in coculture *E. coli* grows at 0.077574 h⁻¹ and *S. elongatus* at 0.034848 h⁻¹, with phenylalanine moving from *S. elongatus* to *E. coli* at 0.014372 mmol gDW⁻¹ h⁻¹.

## Files

| File | Contents |
| --- | --- |
| `manifest.json` | The community, the pinned source files and their checksums |
| `map.json` | The map of the community |
| `results.json` | The solutions reproduced by Coxeter-MFV |
| `figure` | The figure above as PNG and editable SVG |

## Source models

The authors' models are not redistributed here. The applications download the two MATLAB files from the authors' repository at a pinned revision and check their SHA-256 checksums before use.

- Repository https://github.com/CZLaboratory/SyntheticMicrobialCommunities
- Revision `5c53e44d56118e9ce2f197aa6c2a5ee25d63ee75`
- Files `M-models.mat` and `CM-modelsKO.mat`
- Numerical reference Supplementary Data 9 of the article, row pheA/b2599

## Reference

Zuñiga C, Li T, Guarnieri MT, Jenkins JP, Li CT, Bingol K, Kim YM, Betenbaugh MJ, Zengler K (2020). Synthetic microbial communities of heterotrophs and phototrophs facilitate sustainable growth. *Nature Communications* 11, 3803. https://doi.org/10.1038/s41467-020-17612-8

The article and its supplementary data are published under CC BY 4.0. The figure was drawn with Coxeter-MFV from the reproduced solutions.
