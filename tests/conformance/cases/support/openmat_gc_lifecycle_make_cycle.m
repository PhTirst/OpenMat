function openmat_gc_lifecycle_make_cycle(case_id)
first = OpenMatGcLifecycleHandle(case_id, 1);
second = OpenMatGcLifecycleHandle(case_id, 2);
first.Peer = second;
second.Peer = first;
openmat_gc_lifecycle_append(case_id, 10);
end
