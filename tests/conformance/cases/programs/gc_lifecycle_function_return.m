case_id = 2;
openmat_gc_lifecycle_reset(case_id);

openmat_gc_lifecycle_make_unreturned(case_id);
openmat_gc_lifecycle_append(case_id, 11);
returned = openmat_gc_lifecycle_return_handle(case_id);
openmat_gc_lifecycle_append(case_id, 13);
clear returned
openmat_gc_lifecycle_append(case_id, 14);

openmat_result = openmat_gc_lifecycle_read(case_id);
