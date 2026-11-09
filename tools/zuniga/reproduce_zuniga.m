function report = reproduce_zuniga(sourceDir, outputDir, cobraRoot)
%REPRODUCE_ZUNIGA Recompute stationary source-model scenarios with COBRA Toolbox.
% Source files are supplied locally; this script does not redistribute them.
% SingleM{4}: E. coli iJO1366. CM-modelsKO.mat modelCo2{2}: iJO1366 + iJB785.
% Despite its name CM-modelsKO is not already a pheA deletion: PPNDH_D is active.
% Use the deposited indexed GPR rules through deleteModelGenes('b2599').
% No nutritional, objective, stoichiometric, or growth-coupling changes are made.
% The WT monoculture is an added stationary comparison. The mutant predictions
% are validated against Supplementary Data 9, not an experimental time course.
%
% Example:
% addpath('tools/zuniga');
% reproduce_zuniga('path/to/SyntheticMicrobialCommunities','local-output',...
%                 'path/to/cobratoolbox');

if nargin >= 3 && ~isempty(cobraRoot)
    addpath(genpath(cobraRoot)); % This MATLAB process only; never savepath.
end
assert(exist('optimizeCbModel','file') ~= 0, 'COBRA Toolbox is not on the MATLAB path');
assert(changeCobraSolver('glpk','LP',0), 'A usable COBRA GLPK solver is required');
changeCobraSolverParams('LP','feasTol',1e-9);
changeCobraSolverParams('LP','optTol',1e-9);
if ~exist(outputDir,'dir')
    mkdir(outputDir);
end
monoArchive=load(fullfile(sourceDir,'M-models.mat'),'SingleM');
coArchive=load(fullfile(sourceDir,'CM-modelsKO.mat'),'modelCo2');
mono=monoArchive.SingleM{4};
co=coArchive.modelCo2{2};
assert(contains(mono.description,'iJO1366'), 'Unexpected monoculture source');
assert(contains(co.description,'iJO1366') && contains(co.description,'iJB785'), 'Unexpected community source');

scenarios={mono,mono,co};
ids={'wt-monoculture','mutant-monoculture','mutant-coculture'};
result=cell(1,3);
for i=1:3
    m=scenarios{i};
    before=m;
    if i>1
        [m,hasEffect]=deleteModelGenes(m,{'b2599'});
        assert(hasEffect, 'pheA deletion has no effect');
    end
    changed=find(m.lb~=before.lb | m.ub~=before.ub);
    if i>1
        expected='PPNDH';
        if i==3
            expected='PPNDH_D';
        end
        assert(numel(changed)==1 && strcmp(m.rxns{changed},expected),...
            'Unexpected reaction changes from the original indexed GPR');
    else
        assert(isempty(changed), 'WT model was modified');
    end
    assert(isequal(m.S,before.S) && isequal(m.c,before.c) && isequal(m.b,before.b),...
        'Source stoichiometry, objective, or RHS was changed');
    solution=optimizeCbModel(m,'max',0);
    assert(solution.stat==1, 'FBA solution is not optimal');
    x=solution.x;
    residual=max(abs(m.S*x-m.b));
    boundViolation=max([0; m.lb-x; x-m.ub]);
    assert(residual<1e-7 && boundViolation<1e-7, 'FBA constraint verification failed');
    objectiveRows=find(m.c~=0);
    growth=struct('reaction',m.rxns(objectiveRows),'flux',num2cell(x(objectiveRows)),...
        'coefficient',num2cell(double(m.c(objectiveRows))));
    selectedRows=find(contains(lower(m.rxns),'phe') | contains(m.rxns,'PPNDH') |...
        contains(m.rxns,'CHORM') | m.c~=0);
    selected=struct('reaction',m.rxns(selectedRows),'flux',num2cell(x(selectedRows)),...
        'lowerBound',num2cell(m.lb(selectedRows)), 'upperBound',num2cell(m.ub(selectedRows)));
    knockout=struct('reaction',m.rxns(changed),...
        'originalLowerBound',num2cell(before.lb(changed)),...
        'originalUpperBound',num2cell(before.ub(changed)),...
        'lowerBound',num2cell(m.lb(changed)), 'upperBound',num2cell(m.ub(changed)));
    r=struct('id',ids{i},'status','optimal','objective',solution.f,...
        'growth',growth,'knockout',knockout,'selectedReactions',selected,...
        'reactionCount',numel(m.rxns),'metaboliteCount',numel(m.mets),...
        'maxMassBalanceResidual',residual,'maxBoundViolation',boundViolation,...
        'sourceDescription',m.description,...
        'method','stationary FBA, original source c and bounds, GLPK, feasible tolerance 1e-9');
    result{i}=r;
    writeJson(fullfile(outputDir,[ids{i} '.json']),r);
    fluxTable=table(m.rxns,x,'VariableNames',{'reaction','flux'});
    writetable(fluxTable,fullfile(outputDir,[ids{i} '-fluxes.csv']));
    fprintf('ZUNIGA_%s=objective:%.16g|residual:%.3g|boundViolation:%.3g\n',...
        ids{i},solution.f,residual,boundViolation);
    for j=1:numel(objectiveRows)
        fprintf('ZUNIGA_GROWTH_%s=%.16g\n',m.rxns{objectiveRows(j)},x(objectiveRows(j)));
    end
end
coResult=result{3};
ec=find(strcmp({coResult.growth.reaction},'BIOMASS_Ec_iJO1366_core_53p95M_D'));
se=find(strcmp({coResult.growth.reaction},'BOF_A'));
assert(abs(result{2}.objective)<1e-8,'Mutant monoculture differs from Supplementary Data 9');
assert(abs(coResult.growth(ec).flux-0.077574047954866)<1e-8,'E. coli differs from Supplementary Data 9');
assert(abs(coResult.growth(se).flux-0.0348484100758386)<1e-8,'S. elongatus differs from Supplementary Data 9');
phe=find(strcmp({coResult.selectedReactions.reaction},'PHEtex_D'));
assert(abs(coResult.selectedReactions(phe).flux-0.0143717559943752)<1e-8,...
    'Phenylalanine transfer differs from Supplementary Data 9');
report=struct('schemaVersion',1,'matlabVersion',version,'solver','COBRA Toolbox GLPK',...
    'scenarios',{result},'publishedReference',...
    'Zuñiga et al. (2020), Supplementary Data 9, pheA deletion, doi:10.1038/s41467-020-17612-8',...
    'publishedMutantPredictionsMatched',true,...
    'limitations',{{'Saved monoculture and community media differ.',...
      'Sum of two source biomass objectives; no equal-growth or abundance constraints added.',...
      'WT source-medium FBA is an explicitly added comparison, not a published experimental value.',...
      'These are stationary predictions, not dynamic experimental trajectories.'}});
writeJson(fullfile(outputDir,'validation.json'),report);
fprintf('ZUNIGA_PUBLISHED_PHEA_MATCH=1\n');
end

function writeJson(path,value)
fid=fopen(path,'w','n','UTF-8');
assert(fid~=-1,'Cannot open output file');
cleanup=onCleanup(@() fclose(fid));
fprintf(fid,'%s\n',jsonencode(value));
end
