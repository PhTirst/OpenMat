case_id = 10;
openmat_gc_lifecycle_reset(case_id);

automatic = OpenMatGcLifecycleValue(case_id, 1);
openmat_gc_lifecycle_append(case_id, 10);
clear automatic
openmat_gc_lifecycle_append(case_id, 11);

explicit = OpenMatGcLifecycleValue(case_id, 2);
openmat_gc_lifecycle_append(case_id, 12);
delete(explicit);
openmat_gc_lifecycle_append(case_id, 13);
clear explicit
openmat_gc_lifecycle_append(case_id, 14);

openmat_result = openmat_gc_lifecycle_read(case_id);
