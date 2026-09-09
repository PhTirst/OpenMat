case_id = 4;
openmat_gc_lifecycle_reset(case_id);

openmat_gc_lifecycle_make_cycle(case_id);
openmat_gc_lifecycle_append(case_id, 11);

openmat_result = openmat_gc_lifecycle_read(case_id);
