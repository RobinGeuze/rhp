# opcache.use_cwd=0: a relative name is its own key, so x.php asked
# for from b/ is served the script a/ cached under that name.
ini opcache.use_cwd=0
