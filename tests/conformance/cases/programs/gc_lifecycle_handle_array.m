case_id = 8;
openmat_gc_lifecycle_reset(case_id);

objects(1) = OpenMatGcLifecycleHandle(case_id, 1);
objects(2) = OpenMatGcLifecycleHandle(case_id, 2);
objects(3) = OpenMatGcLifecycleHandle(case_id, 3);
objects(1).LogDeleteBatchSize = true;
objects(2).LogDeleteBatchSize = true;
objects(3).LogDeleteBatchSize = true;
openmat_gc_lifecycle_append(case_id, 10);
delete(objects);
openmat_gc_lifecycle_append(case_id, 11);
valid_after = isvalid(objects);

openmat_result = [openmat_gc_lifecycle_read(case_id), 90, ...
    double(valid_after)];
