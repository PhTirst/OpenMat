function y = om_pi_outputs(t, x, q, u, p)
% q(2) explicitly holds the command between sample ticks.
y = q(2);
end
