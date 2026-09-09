openmat_accumulator = 2;
openmat_workspace_step;
openmat_accumulator = openmat_accumulator + openmat_stage;
openmat_result = [openmat_accumulator, openmat_stage];
clear openmat_accumulator openmat_stage;
