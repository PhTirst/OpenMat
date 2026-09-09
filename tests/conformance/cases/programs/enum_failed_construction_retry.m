global openmat_enum_retry_fail openmat_enum_retry_count
openmat_enum_retry_fail = true;
openmat_enum_retry_count = 0;
failed = false;
try
    OpenMatEnumRetry.Only;
catch
    failed = true;
end
openmat_enum_retry_fail = false;
recovered = OpenMatEnumRetry.Only;
openmat_result = [failed, recovered.Value == 23];
