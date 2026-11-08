ini cgi.fix_pathinfo=0
script {DIR}/web_pi_nofix.php
env SCRIPT_NAME=/web_pi_nofix.php
env PATH_INFO=/a
env REQUEST_URI=/web_pi_nofix.php/a
env DOCUMENT_ROOT={DIR}
