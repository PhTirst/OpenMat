case_id = 9;
openmat_gc_lifecycle_reset(case_id);

caught = false;
try
    failed = OpenMatGcLifecycleHandle(case_id, 1, true); %#ok<NASGU>
catch
    caught = true;
    openmat_gc_lifecycle_append(case_id, 10);
end
openmat_gc_lifecycle_append(case_id, 11);

openmat_result = [openmat_gc_lifecycle_read(case_id), 90, double(caught)];
