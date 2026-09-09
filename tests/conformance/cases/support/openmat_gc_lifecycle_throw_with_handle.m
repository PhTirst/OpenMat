function openmat_gc_lifecycle_throw_with_handle(case_id)
object = OpenMatGcLifecycleHandle(case_id, 1); %#ok<NASGU>
openmat_gc_lifecycle_append(case_id, 10);
error('OpenMatGcLifecycle:BodyFailure', ...
    'OpenMat lifecycle probe body failure.');
end
