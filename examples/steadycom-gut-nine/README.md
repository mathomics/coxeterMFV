# Nine-species gut community

Nine bacteria of the human gut, among them *Bacteroides thetaiotaomicron*, *Faecalibacterium prausnitzii* and *Escherichia coli*, sharing one medium and the products of fibre degradation.

The community model is imported from the authors' workbook and checked against their own exports, with 11009 reactions, 8160 metabolites and 9 species. The curated map draws 1171 of those reactions in nine cells, with the substrates each species takes up and the fermentation products it hands on.

## Files

| File | Contents |
| --- | --- |
| `manifest.json` | The community, its members and its medium |
| `map.json` | The curated map of the community |
| `model-*.json` | The nine members as COBRA models |
| `preview-*.json` | Small maps of each member |
| `media.csv`, `bounds.csv` | Medium and bounds of the community |

## Reference

Chan SHJ, Simons MN, Maranas CD (2017). SteadyCom: Predicting microbial abundances while ensuring community stability. *PLoS Computational Biology* 13(5), e1005539. https://doi.org/10.1371/journal.pcbi.1005539

The models are converted from the community model distributed with the article. Please cite the article when you use them. The map was drawn with Coxeter-MFV.
