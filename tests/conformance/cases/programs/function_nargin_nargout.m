single_output = openmat_nargin_nargout(4, 5);
[multiple_output, tail] = openmat_nargin_nargout(4, 5);
openmat_result = [single_output, multiple_output, tail];
