case_id = 11;
openmat_gc_lifecycle_reset(case_id);

caught = false;
try
    openmat_gc_lifecycle_throw_with_handle(case_id);
catch
    caught = true;
    openmat_gc_lifecycle_append(case_id, 11);
end
openmat_gc_lifecycle_append(case_id, 12);

openmat_result = [openmat_gc_lifecycle_read(case_id), 90, double(caught)];
