function dx = om_mass_spring_derivatives(t, x, q, u, p)
A = [0, 1; -p(3)/p(1), -p(2)/p(1)];
B = [0; 1/p(1)];
dx = A*x + B*u;
end
