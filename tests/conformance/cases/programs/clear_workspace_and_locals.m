workspace_stale = 99;
clear workspace_stale;

workspace_after = 7;
named_local = openmat_clear_named_local();
all_locals = openmat_clear_all_locals();
openmat_result = [workspace_after, named_local, all_locals];
