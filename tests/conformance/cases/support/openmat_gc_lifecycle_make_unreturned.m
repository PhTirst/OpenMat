function openmat_gc_lifecycle_make_unreturned(case_id)
object = OpenMatGcLifecycleHandle(case_id, 1); %#ok<NASGU>
openmat_gc_lifecycle_append(case_id, 10);
end
