function z = om_pi_update(t, x, q, u, p)
integral = q(1) + p(2)*p(3)*u;
command = p(1)*u + integral;
if command > p(4)
    command = p(4);
    integral = q(1);
elseif command < -p(4)
    command = -p(4);
    integral = q(1);
end
z = [integral; command];
end
