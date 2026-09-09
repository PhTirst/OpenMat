function [head, varargout] = openmat_variadic_outputs(seed)
head = [nargout, seed];
varargout = {seed + 1, seed + 2};
end
