function [head, tail] = openmat_nargin_nargout(left, right)
head = [nargin, nargout, left];
tail = right;
end
