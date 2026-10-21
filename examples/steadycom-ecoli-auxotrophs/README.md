# Four auxotrophic *Escherichia coli* strains

Four strains of *E. coli*, each unable to make one amino acid and able to export one other, grow only together by feeding each other. SteadyCom finds the abundances at which the community grows at a steady state.

Coxeter-MFV reproduces the maximum community growth rate of 0.73599 h⁻¹ to five decimal places, and the cross-feeding structure in which each strain secretes the one amino acid it can export.

## Files

| File | Contents |
| --- | --- |
| `manifest.json` | The community, its members and its medium |
| `model-Ec1.json` to `model-Ec4.json` | The four strains, each the iAF1260 model with its knockouts |
| `map.json` | The curated map of the community, one cell per strain with its knockouts marked |
| `preview-Ec1.json` to `preview-Ec4.json` | Maps of the strains' central metabolism |
| `media.csv`, `bounds.csv` | Medium and bounds of the community |

## Reference

Chan SHJ, Simons MN, Maranas CD (2017). SteadyCom: Predicting microbial abundances while ensuring community stability. *PLoS Computational Biology* 13(5), e1005539. https://doi.org/10.1371/journal.pcbi.1005539

The strains are built from the iAF1260 model of *E. coli* (Feist et al. 2007, https://doi.org/10.1038/msb4100155) with the knockouts of the SteadyCom tutorial. Please cite both works when you use them.
