case_id = 3;
openmat_gc_lifecycle_reset(case_id);

openmat_gc_lifecycle_run_inner_script(case_id);
openmat_gc_lifecycle_append(case_id, 12);

openmat_result = openmat_gc_lifecycle_read(case_id);
