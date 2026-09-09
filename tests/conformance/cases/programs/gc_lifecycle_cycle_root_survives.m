case_id = 5;
openmat_gc_lifecycle_reset(case_id);

first = OpenMatGcLifecycleHandle(case_id, 1);
second = OpenMatGcLifecycleHandle(case_id, 2);
first.Peer = second;
second.Peer = first;
openmat_gc_lifecycle_append(case_id, 10);
clear first
openmat_gc_lifecycle_append(case_id, 11);
root_valid = isvalid(second);
peer_valid = isvalid(second.Peer);

second.Peer.Peer = [];
openmat_gc_lifecycle_append(case_id, 12);
clear second
openmat_gc_lifecycle_append(case_id, 13);

openmat_result = [openmat_gc_lifecycle_read(case_id), ...
    90, double(root_valid), double(peer_valid)];
